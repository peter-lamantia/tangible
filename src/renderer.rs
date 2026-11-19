use crate::loading::AppEvent;
use crate::{camera, mesh, theme};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, Arc,
};
use wgpu::util::DeviceExt;
use winit::event_loop::EventLoopProxy;
use winit::window::Window;
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    mvp: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    normal_matrix: [[f32; 4]; 4],
    light_dir: [f32; 3],
    _pad1: f32,
    light_color: [f32; 3],
    _pad2: f32,
    ambient: [f32; 3],
    _pad3: f32,
    eye_pos: [f32; 3],
    edges: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PackedMaterial {
    surface: [f32; 4],
    emissive: [f32; 3],
    map_factors: [f32; 2],
}
impl From<mesh::Material> for PackedMaterial {
    fn from(material: mesh::Material) -> Self {
        Self {
            surface: [
                material.metallic,
                material.roughness,
                match material.alpha {
                    mesh::AlphaMode::Opaque => -2.,
                    mesh::AlphaMode::Blend => -1.,
                    mesh::AlphaMode::Mask(cutoff) => cutoff,
                },
                f32::from(material.unlit),
            ],
            emissive: material.emissive,
            map_factors: material.map_factors,
        }
    }
}

#[derive(Debug)]
struct GeometryChunk {
    vertices: wgpu::Buffer,
    material: wgpu::Buffer,
    indices: Option<wgpu::Buffer>,
    count: u32,
    topology: usize,
    texture: usize,
    blend: bool,
    center: glam::Vec3,
}

#[derive(Debug)]
pub(crate) struct PreparedMesh {
    revision: u64,
    mesh: mesh::Mesh,
    geometry: Vec<GeometryChunk>,
    textures: Vec<wgpu::BindGroup>,
}
struct UploadJob {
    revision: u64,
    mesh: mesh::Mesh,
    cached: Option<Vec<GeometryChunk>>,
}

struct UploadWorker {
    jobs: Option<mpsc::Sender<UploadJob>>,
    thread: Option<std::thread::JoinHandle<()>>,
    latest: Arc<AtomicU64>,
}
impl UploadWorker {
    fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        layout: Arc<wgpu::BindGroupLayout>,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Self {
        Self::with_delivery(device, queue, layout, move |prepared| {
            proxy.send_event(AppEvent::GpuUploadReady(prepared)).is_ok()
        })
    }
    fn with_delivery(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        layout: Arc<wgpu::BindGroupLayout>,
        deliver: impl Fn(PreparedMesh) -> bool + Send + 'static,
    ) -> Self {
        let (jobs, receiver) = mpsc::channel::<UploadJob>();
        let latest = Arc::new(AtomicU64::new(0));
        let current = latest.clone();
        let thread = std::thread::spawn(move || {
            while let Ok(mut job) = receiver.recv() {
                // Keep only the newest queued mesh (including rapid plate changes).
                while let Ok(next) = receiver.try_recv() {
                    job = next;
                }
                let is_current = || current.load(Ordering::SeqCst) == job.revision;
                if !is_current() {
                    continue;
                }
                let upload_start = std::time::Instant::now();
                let geometry = match job.cached {
                    Some(geometry) => geometry,
                    None => match GpuState::create_vertex_buffers_cancellable(
                        &device, &job.mesh, is_current,
                    ) {
                        Some(geometry) => geometry,
                        None => continue,
                    },
                };
                let Some(textures) = GpuState::create_textures_cancellable(
                    &device, &queue, &layout, &job.mesh, is_current,
                ) else {
                    continue;
                };
                if !is_current() {
                    continue;
                }
                // Flush texture writes here rather than leaving their submission to the UI.
                queue.submit([]);
                log::debug!(
                    "Background geometry/texture preparation: {:?} ({:.1} MiB of vertex and index buffers, {} indexed chunks out of {})",
                    upload_start.elapsed(),
                    geometry.iter().map(|chunk| chunk.vertices.size() + chunk.indices.as_ref().map_or(0, wgpu::Buffer::size)).sum::<u64>() as f64 / (1024. * 1024.),
                    geometry.iter().filter(|chunk| chunk.indices.is_some()).count(),
                    geometry.len()
                );
                let prepared = PreparedMesh {
                    revision: job.revision,
                    mesh: job.mesh,
                    geometry,
                    textures,
                };
                if !deliver(prepared) {
                    break;
                }
            }
        });
        Self {
            jobs: Some(jobs),
            thread: Some(thread),
            latest,
        }
    }
    fn send(&self, job: UploadJob) -> Result<(), mpsc::SendError<UploadJob>> {
        self.jobs
            .as_ref()
            .expect("upload worker is running")
            .send(job)
    }
    fn cancel(&self) {
        self.latest.fetch_add(1, Ordering::SeqCst);
    }
}
impl Drop for UploadWorker {
    fn drop(&mut self) {
        self.cancel();
        // Disconnect the channel before joining so an idle worker can wake up.
        drop(self.jobs.take());
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                log::error!("GPU upload worker panicked");
            }
        }
    }
}

pub(crate) struct GpuState {
    // Drop the worker before the active graphics resources.
    upload_worker: UploadWorker,
    surface: wgpu::Surface<'static>,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pub(crate) config: wgpu::SurfaceConfiguration,
    pipeline: Vec<wgpu::RenderPipeline>,
    texture_layout: Arc<wgpu::BindGroupLayout>,
    uploading: bool,
    textures: Vec<wgpu::BindGroup>,
    animation: Option<Arc<crate::model::gltf::Scene>>,
    pub(crate) animation_paused: bool,
    animation_time: f32,
    last_animation_frame: std::time::Instant,
    vertex_buffers: Vec<GeometryChunk>,
    mesh_storage: Arc<Vec<mesh::Vertex>>,
    geometry_cache: std::collections::VecDeque<(Arc<Vec<mesh::Vertex>>, Vec<GeometryChunk>)>,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    depth_view: wgpu::TextureView,
    msaa_view: Option<wgpu::TextureView>,
    pub(crate) sample_count: u32,
    pub(crate) msaa_supported: bool,
    pub(crate) show_edges: bool,
    shader: wgpu::ShaderModule,
    bind_group_layout: wgpu::BindGroupLayout,
    egui_renderer: egui_wgpu::Renderer,
    num_vertices: u32,
}

impl GpuState {
    pub(crate) fn new(
        window: Arc<Window>,
        mesh: &mesh::Mesh,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Result<Self, String> {
        let size = window.inner_size();

        // ── Instance + Surface + Adapter + Device ────────────────────────
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("Could not create graphics surface: {e}"))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .ok_or(
            "No suitable graphics adapter found. Check that your graphics drivers are installed.",
        )?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
            },
            None,
        ))
        .map_err(|e| format!("Could not initialize graphics device: {e}"))?;

        let device = Arc::new(device);
        let queue = Arc::new(queue);

        // ── Surface config ───────────────────────────────────────────────
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .or_else(|| caps.formats.first().copied())
            .ok_or("No supported display format found")?;

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: *caps
                .alpha_modes
                .first()
                .ok_or("No supported display alpha mode found")?,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let msaa_supported = [format, wgpu::TextureFormat::Depth32Float]
            .iter()
            .all(|&format| {
                adapter
                    .get_texture_format_features(format)
                    .flags
                    .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4)
            });

        // ── Shader ───────────────────────────────────────────────────────
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        // ── Buffers ──────────────────────────────────────────────────────
        let vertex_buffers = Self::create_vertex_buffers(&device, mesh);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uniform Buffer"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Bind group ───────────────────────────────────────────────────
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let texture_layout = Arc::new(Self::texture_bind_group_layout(&device));
        let textures = Self::create_textures(&device, &queue, &texture_layout, mesh);
        let sample_count = 1;
        let pipeline = Self::create_pipeline(
            &device,
            &config,
            &shader,
            &bind_group_layout,
            &texture_layout,
            sample_count,
        );
        let depth_view = Self::create_depth_texture(&device, &config, sample_count);
        let msaa_view = None;

        let egui_renderer = egui_wgpu::Renderer::new(&device, config.format, None, 1, false);

        let upload_worker =
            UploadWorker::new(device.clone(), queue.clone(), texture_layout.clone(), proxy);
        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            texture_layout,
            upload_worker,
            uploading: false,
            textures,
            animation: mesh.animation.clone(),
            animation_paused: false,
            animation_time: 0.,
            last_animation_frame: std::time::Instant::now(),
            vertex_buffers,
            mesh_storage: mesh.vertices.clone(),
            geometry_cache: Default::default(),
            uniform_buffer,
            bind_group,
            depth_view,
            msaa_view,
            sample_count,
            msaa_supported,
            show_edges: false,
            shader,
            bind_group_layout,
            egui_renderer,
            num_vertices: mesh.vertices.len() as u32,
        })
    }

    fn create_pipeline(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        shader: &wgpu::ShaderModule,
        bind_group_layout: &wgpu::BindGroupLayout,
        texture_layout: &wgpu::BindGroupLayout,
        sample_count: u32,
    ) -> Vec<wgpu::RenderPipeline> {
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Pipeline Layout"),
            bind_group_layouts: &[bind_group_layout, texture_layout],
            push_constant_ranges: &[],
        });
        (0..12)
            .map(|variant| {
                let topology = [
                    wgpu::PrimitiveTopology::TriangleList,
                    wgpu::PrimitiveTopology::LineList,
                    wgpu::PrimitiveTopology::PointList,
                ][variant % 3];
                let cull = variant / 3 % 2 != 0;
                let blend = variant / 6 != 0;
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("Render Pipeline"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: shader,
                        entry_point: "vs_main",
                        buffers: &[
                            wgpu::VertexBufferLayout {
                                array_stride: std::mem::size_of::<mesh::Vertex>() as u64,
                                step_mode: wgpu::VertexStepMode::Vertex,
                                attributes: &[
                                    wgpu::VertexAttribute {
                                        offset: 0,
                                        shader_location: 0,
                                        format: wgpu::VertexFormat::Float32x3,
                                    },
                                    wgpu::VertexAttribute {
                                        offset: 12,
                                        shader_location: 1,
                                        format: wgpu::VertexFormat::Float32x3,
                                    },
                                    wgpu::VertexAttribute {
                                        offset: 24,
                                        shader_location: 2,
                                        format: wgpu::VertexFormat::Float32x4,
                                    },
                                    wgpu::VertexAttribute {
                                        offset: 40,
                                        shader_location: 3,
                                        format: wgpu::VertexFormat::Float32x2,
                                    },
                                    wgpu::VertexAttribute {
                                        offset: 48,
                                        shader_location: 6,
                                        format: wgpu::VertexFormat::Float32x2,
                                    },
                                    wgpu::VertexAttribute {
                                        offset: 56,
                                        shader_location: 7,
                                        format: wgpu::VertexFormat::Float32x2,
                                    },
                                    wgpu::VertexAttribute {
                                        offset: 64,
                                        shader_location: 8,
                                        format: wgpu::VertexFormat::Float32x2,
                                    },
                                    wgpu::VertexAttribute {
                                        offset: 72,
                                        shader_location: 9,
                                        format: wgpu::VertexFormat::Float32x2,
                                    },
                                    wgpu::VertexAttribute {
                                        offset: 80,
                                        shader_location: 11,
                                        format: wgpu::VertexFormat::Float32x4,
                                    },
                                ],
                            },
                            wgpu::VertexBufferLayout {
                                array_stride: std::mem::size_of::<PackedMaterial>() as u64,
                                step_mode: wgpu::VertexStepMode::Instance,
                                attributes: &[
                                    wgpu::VertexAttribute {
                                        offset: 0,
                                        shader_location: 4,
                                        format: wgpu::VertexFormat::Float32x4,
                                    },
                                    wgpu::VertexAttribute {
                                        offset: 16,
                                        shader_location: 5,
                                        format: wgpu::VertexFormat::Float32x3,
                                    },
                                    wgpu::VertexAttribute {
                                        offset: 28,
                                        shader_location: 10,
                                        format: wgpu::VertexFormat::Float32x2,
                                    },
                                ],
                            },
                        ],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: shader,
                        entry_point: "fs_main",
                        targets: &[Some(wgpu::ColorTargetState {
                            format: config.format,
                            blend: Some(if blend {
                                wgpu::BlendState::ALPHA_BLENDING
                            } else {
                                wgpu::BlendState::REPLACE
                            }),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology,
                        strip_index_format: None,
                        front_face: wgpu::FrontFace::Ccw,
                        cull_mode: cull.then_some(wgpu::Face::Back),
                        polygon_mode: wgpu::PolygonMode::Fill,
                        unclipped_depth: false,
                        conservative: false,
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: !blend,
                        depth_compare: wgpu::CompareFunction::Less,
                        stencil: wgpu::StencilState::default(),
                        bias: wgpu::DepthBiasState::default(),
                    }),
                    multisample: wgpu::MultisampleState {
                        count: sample_count,
                        ..Default::default()
                    },
                    multiview: None,
                    cache: None,
                })
            })
            .collect()
    }

    fn create_vertex_buffers(device: &wgpu::Device, mesh: &mesh::Mesh) -> Vec<GeometryChunk> {
        Self::create_vertex_buffers_cancellable(device, mesh, || true).unwrap()
    }
    fn create_vertex_buffers_cancellable(
        device: &wgpu::Device,
        mesh: &mesh::Mesh,
        is_current: impl Fn() -> bool,
    ) -> Option<Vec<GeometryChunk>> {
        // Bound temporary deduplication memory as well as device buffer sizes.
        let chunk_size = vertex_chunk_size(device.limits().max_buffer_size).min(196_608);
        let batches = if mesh.batches.is_empty() {
            vec![mesh::Batch {
                range: 0..mesh.vertices.len(),
                source_indices: None,
                material: Default::default(),
                topology: mesh::Topology::Triangles,
                texture: None,
                maps: [None; 4],
                double_sided: true,
                blend: false,
            }]
        } else {
            mesh.batches.clone()
        };
        batches
            .iter()
            .enumerate()
            .flat_map(|(batch_index, batch)| {
                mesh.vertices[batch.range.clone()]
                    .chunks(chunk_size)
                    .enumerate()
                    .map(move |(i, vertices)| (batch_index, batch, vertices, i * chunk_size))
            })
            .map(|(batch_index, batch, vertices, offset)| {
                if !is_current() {
                    return None;
                }
                let (unique, indices) = if mesh.animation.is_some() {
                    (Vec::new(), Vec::new())
                } else if let Some(source) = &batch.source_indices {
                    indexed_source_vertices(vertices, &source[offset..offset + vertices.len()])
                } else if should_deduplicate(vertices) {
                    indexed_vertices(vertices)
                } else {
                    (Vec::new(), Vec::new())
                };
                let indexed =
                    !indices.is_empty() && indexing_saves_space(unique.len(), &indices, vertices);
                let data = if indexed { unique.as_slice() } else { vertices };
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Model vertices"),
                    contents: bytemuck::cast_slice(data),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                });
                let index_buffer = indexed.then(|| {
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Model indices"),
                        contents: bytemuck::cast_slice(&indices),
                        usage: wgpu::BufferUsages::INDEX,
                    })
                });
                Some(GeometryChunk {
                    vertices: buffer,
                    material: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Material"),
                        contents: bytemuck::bytes_of(&PackedMaterial::from(batch.material)),
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    }),
                    indices: index_buffer,
                    count: vertices.len() as u32,
                    topology: batch.topology as usize
                        + usize::from(!batch.double_sided) * 3
                        + usize::from(batch.blend) * 6,
                    texture: batch_index,
                    blend: batch.blend,
                    center: if batch.blend {
                        chunk_center(vertices)
                    } else {
                        glam::Vec3::ZERO
                    },
                })
            })
            .collect()
    }

    pub(crate) fn load_mesh(&mut self, mesh: &mesh::Mesh) {
        let revision = self.upload_worker.latest.fetch_add(1, Ordering::SeqCst) + 1;
        let cached = self
            .geometry_cache
            .iter()
            .position(|(storage, _)| Arc::ptr_eq(storage, &mesh.vertices))
            .and_then(|i| self.geometry_cache.remove(i))
            .map(|(_, buffers)| buffers);
        self.uploading = self
            .upload_worker
            .send(UploadJob {
                revision,
                mesh: mesh.clone(),
                cached,
            })
            .is_ok();
    }

    pub(crate) fn cancel_upload(&mut self) {
        self.upload_worker.cancel();
        self.uploading = false;
    }
    pub(crate) fn is_loading(&self) -> bool {
        self.uploading
    }

    pub(crate) fn accept_upload(&mut self, prepared: PreparedMesh) -> bool {
        if prepared.revision != self.upload_worker.latest.load(Ordering::SeqCst) {
            return false;
        }
        let previous = std::mem::replace(&mut self.vertex_buffers, prepared.geometry);
        if self.mesh_storage.len() * std::mem::size_of::<mesh::Vertex>() <= 32 * 1024 * 1024 {
            self.geometry_cache
                .push_back((self.mesh_storage.clone(), previous));
        }
        while self.geometry_cache.len() > 2 {
            self.geometry_cache.pop_front();
        }
        self.animation = prepared.mesh.animation;
        self.animation_time = 0.;
        self.last_animation_frame = std::time::Instant::now();
        self.animation_paused = false;
        self.mesh_storage = prepared.mesh.vertices;
        self.num_vertices = self.mesh_storage.len() as u32;
        self.textures = prepared.textures;
        self.uploading = false;
        true
    }

    fn texture_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        let entries: Vec<_> = (0..10)
            .map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: if binding % 2 == 0 {
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    }
                } else {
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)
                },
                count: None,
            })
            .collect();
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Material texture layout"),
            entries: &entries,
        })
    }

    fn create_textures(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        mesh: &mesh::Mesh,
    ) -> Vec<wgpu::BindGroup> {
        Self::create_textures_cancellable(device, queue, layout, mesh, || true).unwrap()
    }
    fn create_textures_cancellable(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        mesh: &mesh::Mesh,
        is_current: impl Fn() -> bool,
    ) -> Option<Vec<wgpu::BindGroup>> {
        let white = mesh::Texture {
            width: 1,
            height: 1,
            pixels: vec![255; 4],
            wrap: [1; 2],
            linear: [true; 2],
        };
        let normal = mesh::Texture {
            pixels: vec![128, 128, 255, 255],
            ..white.clone()
        };
        let maps: Vec<[usize; 5]> = if mesh.batches.is_empty() {
            vec![[0, 0, 1, 0, 0]]
        } else {
            mesh.batches
                .iter()
                .map(|b| {
                    let maps = [b.texture, b.maps[0], b.maps[1], b.maps[2], b.maps[3]];
                    std::array::from_fn(|slot| {
                        maps[slot].map_or(usize::from(slot == 2), |index| index + 2)
                    })
                })
                .collect()
        };
        // Color maps need sRGB decoding and data maps need linear sampling.
        // Allocate in the required format directly for adapters without texture view formats.
        let is_srgb = |slot| usize::from(slot == 0 || slot == 4);
        let formats = [
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ];
        let mut used_formats = vec![[false; 2]; mesh.textures.len() + 2];
        for indices in &maps {
            for (slot, &image) in indices.iter().enumerate() {
                used_formats[image][is_srgb(slot)] = true;
            }
        }
        let uploaded: Vec<_> = [&white, &normal]
            .into_iter()
            .chain(mesh.textures.iter())
            .zip(used_formats)
            .map(|(image, used)| {
                if !is_current() {
                    return None;
                }
                let size = wgpu::Extent3d {
                    width: image.width,
                    height: image.height,
                    depth_or_array_layers: 1,
                };
                let mut views = [None, None];
                for (index, format) in formats.into_iter().enumerate() {
                    if !used[index] {
                        continue;
                    }
                    if !is_current() {
                        return None;
                    }
                    let texture = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("Material texture"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    queue.write_texture(
                        texture.as_image_copy(),
                        &image.pixels,
                        wgpu::ImageDataLayout {
                            offset: 0,
                            bytes_per_row: Some(image.width * 4),
                            rows_per_image: Some(image.height),
                        },
                        size,
                    );
                    views[index] =
                        Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
                }
                let address = |v| match v {
                    0 => wgpu::AddressMode::ClampToEdge,
                    2 => wgpu::AddressMode::MirrorRepeat,
                    _ => wgpu::AddressMode::Repeat,
                };
                let filter = |linear| {
                    if linear {
                        wgpu::FilterMode::Linear
                    } else {
                        wgpu::FilterMode::Nearest
                    }
                };
                let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                    address_mode_u: address(image.wrap[0]),
                    address_mode_v: address(image.wrap[1]),
                    mag_filter: filter(image.linear[0]),
                    min_filter: filter(image.linear[1]),
                    ..Default::default()
                });
                Some((views, sampler))
            })
            .collect::<Option<Vec<_>>>()?;
        maps.iter()
            .map(|indices| {
                if !is_current() {
                    return None;
                }
                let entries: Vec<_> = (0..10)
                    .map(|binding| {
                        let slot = binding as usize / 2;
                        let (views, sampler) = &uploaded[indices[slot]];
                        wgpu::BindGroupEntry {
                            binding,
                            resource: if binding % 2 == 0 {
                                wgpu::BindingResource::TextureView(
                                    views[is_srgb(slot)]
                                        .as_ref()
                                        .expect("required texture format"),
                                )
                            } else {
                                wgpu::BindingResource::Sampler(sampler)
                            },
                        }
                    })
                    .collect();
                Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Material maps"),
                    layout,
                    entries: &entries,
                }))
            })
            .collect()
    }

    fn ordered_chunks<'a>(
        chunks: &'a [GeometryChunk],
        camera: &camera::Camera,
    ) -> Vec<&'a GeometryChunk> {
        let forward = (camera.target - camera.eye()).normalize_or_zero();
        let mut ordered: Vec<_> = chunks.iter().collect();
        ordered.sort_by(|a, b| {
            a.blend.cmp(&b.blend).then_with(|| {
                if a.blend {
                    b.center.dot(forward).total_cmp(&a.center.dot(forward))
                } else {
                    std::cmp::Ordering::Equal
                }
            })
        });
        ordered
    }

    pub(crate) fn reset_animation_clock(&mut self) {
        self.last_animation_frame = std::time::Instant::now();
    }

    pub(crate) fn has_animation(&self) -> bool {
        self.animation.is_some()
    }

    pub(crate) fn is_animated(&self) -> bool {
        self.animation.is_some() && !self.animation_paused
    }

    fn create_depth_texture(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        sample_count: u32,
    ) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Depth Texture"),
                size: wgpu::Extent3d {
                    width: config.width.max(1),
                    height: config.height.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn create_msaa_view(&self) -> Option<wgpu::TextureView> {
        (self.sample_count > 1).then(|| {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("MSAA Color Texture"),
                    size: wgpu::Extent3d {
                        width: self.config.width,
                        height: self.config.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: self.sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.config.format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        })
    }

    pub(crate) fn set_antialiasing(&mut self, enabled: bool) {
        let sample_count = if enabled && self.msaa_supported { 4 } else { 1 };
        if self.sample_count == sample_count {
            return;
        }
        self.sample_count = sample_count;
        self.pipeline = Self::create_pipeline(
            &self.device,
            &self.config,
            &self.shader,
            &self.bind_group_layout,
            &self.texture_layout,
            sample_count,
        );
        self.depth_view = Self::create_depth_texture(&self.device, &self.config, sample_count);
        self.msaa_view = self.create_msaa_view();
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
            self.depth_view =
                Self::create_depth_texture(&self.device, &self.config, self.sample_count);
            self.msaa_view = self.create_msaa_view();
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        &mut self,
        cam: &camera::Camera,
        egui_ctx: &egui::Context,
        egui_shapes: Vec<egui::epaint::ClippedShape>,
        textures_delta: &egui::TexturesDelta,
        pixels_per_point: f32,
        scene_left: u32,
        is_dark: bool,
    ) {
        let now = std::time::Instant::now();
        if !self.animation_paused {
            self.animation_time += (now - self.last_animation_frame).as_secs_f32();
        }
        self.last_animation_frame = now;
        if let Some(animation) = &self.animation {
            if !self.animation_paused {
                match animation.evaluate(self.animation_time) {
                    Ok(mesh) => {
                        let mut offset = 0;
                        for chunk in &mut self.vertex_buffers {
                            let vertices = &mesh.vertices[offset..offset + chunk.count as usize];
                            self.queue.write_buffer(
                                &chunk.vertices,
                                0,
                                bytemuck::cast_slice(vertices),
                            );
                            self.queue.write_buffer(
                                &chunk.material,
                                0,
                                bytemuck::bytes_of(&PackedMaterial::from(
                                    mesh.batches[chunk.texture].material,
                                )),
                            );
                            if chunk.blend {
                                chunk.center = chunk_center(vertices);
                            }
                            offset += chunk.count as usize;
                        }
                    }
                    Err(error) => {
                        log::error!("Animation failed: {error}");
                        self.animation = None;
                    }
                }
            }
        }
        let output = match self.surface.get_current_texture() {
            Ok(t) => t,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            Err(e) => {
                log::error!("Surface error: {e}");
                return;
            }
        };

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Build 3D uniforms with theme-adaptive ambient and light colors
        let model = glam::Mat4::IDENTITY;
        let view_mat = cam.view_matrix();
        let proj = cam.projection_matrix();
        let mvp = proj * view_mat * model;

        let ambient = if is_dark {
            [0.18, 0.18, 0.22]
        } else {
            [0.34, 0.35, 0.38]
        };

        let light_color = if is_dark {
            [1.0, 0.98, 0.95]
        } else {
            [0.94, 0.95, 0.98]
        };

        let uniforms = Uniforms {
            mvp: mvp.to_cols_array_2d(),
            model: model.to_cols_array_2d(),
            normal_matrix: model.inverse().transpose().to_cols_array_2d(),
            light_dir: glam::Vec3::new(0.5, 1.0, 0.8).normalize().to_array(),
            _pad1: 0.0,
            light_color,
            _pad2: 0.0,
            ambient,
            _pad3: 0.0,
            eye_pos: cam.eye().to_array(),
            edges: if self.show_edges { 1.0 } else { 0.0 },
        };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

        // Egui texture updates
        for (id, delta) in &textures_delta.set {
            self.egui_renderer
                .update_texture(&self.device, &self.queue, *id, delta);
        }

        let clipped_primitives = egui_ctx.tessellate(egui_shapes, pixels_per_point);
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point,
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &clipped_primitives,
            &screen_descriptor,
        );

        // Pass 1: 3D Scene
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3D Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.msaa_view.as_ref().unwrap_or(&view),
                    resolve_target: self.msaa_view.as_ref().map(|_| &view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(theme::clear_color(is_dark)),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            if self.num_vertices > 0 {
                let left = scene_left.min(self.config.width.saturating_sub(1));
                let width = self.config.width - left;
                pass.set_viewport(
                    left as f32,
                    0.0,
                    width as f32,
                    self.config.height as f32,
                    0.0,
                    1.0,
                );
                pass.set_scissor_rect(left, 0, width, self.config.height);

                pass.set_bind_group(0, &self.bind_group, &[]);
                for chunk in Self::ordered_chunks(&self.vertex_buffers, cam) {
                    pass.set_pipeline(&self.pipeline[chunk.topology]);
                    pass.set_bind_group(1, &self.textures[chunk.texture], &[]);
                    pass.set_vertex_buffer(0, chunk.vertices.slice(..));
                    pass.set_vertex_buffer(1, chunk.material.slice(..));
                    if let Some(indices) = &chunk.indices {
                        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..chunk.count, 0, 0..1);
                    } else {
                        pass.draw(0..chunk.count, 0..1);
                    }
                }
            }
        }

        // Pass 2: egui UI overlay
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            self.egui_renderer.render(
                &mut pass.forget_lifetime(),
                &clipped_primitives,
                &screen_descriptor,
            );
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        log::debug!("CPU frame submission: {:?}", now.elapsed());

        for id in &textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
    }
}

// Accessor identity avoids hashing every UV/color attribute. Flat face normals,
// generated tangents, and shader edge corners still need distinct vertices.
fn indexed_source_vertices(
    vertices: &[mesh::Vertex],
    source: &[u32],
) -> (Vec<mesh::Vertex>, Vec<u32>) {
    let mut lookup = std::collections::HashMap::new();
    let mut unique = Vec::new();
    let mut indices = Vec::with_capacity(vertices.len());
    for (i, (vertex, source)) in vertices.iter().zip(source).enumerate() {
        let key = (
            *source,
            vertex.normal.map(f32::to_bits),
            vertex.tangent.map(f32::to_bits),
            i % 3,
        );
        let index = *lookup.entry(key).or_insert_with(|| {
            while unique.len() % 3 != i % 3 {
                unique.push(*vertex);
            }
            let index = unique.len() as u32;
            unique.push(*vertex);
            index
        });
        indices.push(index);
    }
    (unique, indices)
}

fn chunk_center(vertices: &[mesh::Vertex]) -> glam::Vec3 {
    vertices.iter().fold(glam::Vec3::ZERO, |sum, vertex| {
        sum + glam::Vec3::from_array(vertex.position) / vertices.len() as f32
    })
}

fn indexing_saves_space(unique_count: usize, indices: &[u32], expanded: &[mesh::Vertex]) -> bool {
    unique_count * std::mem::size_of::<mesh::Vertex>() + std::mem::size_of_val(indices)
        < std::mem::size_of_val(expanded)
}

fn should_deduplicate(vertices: &[mesh::Vertex]) -> bool {
    const SAMPLE_WINDOWS: usize = 4;
    const WINDOW_VERTICES: usize = 768;
    if vertices.len() <= SAMPLE_WINDOWS * WINDOW_VERTICES * 2 {
        return true;
    }
    // Sample local reuse throughout the chunk. Start on triangle boundaries so
    // the sample uses the same corner identities as the full indexing pass.
    (0..SAMPLE_WINDOWS).any(|window| {
        let start = ((vertices.len() - WINDOW_VERTICES) / 3 * window / (SAMPLE_WINDOWS - 1)) * 3;
        let sample = &vertices[start..start + WINDOW_VERTICES];
        let (unique, indices) = indexed_vertices(sample);
        indexing_saves_space(unique.len(), &indices, sample)
    })
}

// Cap individual uploads at 64 MiB even on devices supporting larger buffers.
fn indexed_vertices(vertices: &[mesh::Vertex]) -> (Vec<mesh::Vertex>, Vec<u32>) {
    let mut lookup = std::collections::HashMap::new();
    let mut unique = Vec::new();
    let mut indices = Vec::with_capacity(vertices.len());
    for (i, vertex) in vertices.iter().enumerate() {
        let key = (
            vertex.position.map(f32::to_bits),
            vertex.normal.map(f32::to_bits),
            vertex.color.map(f32::to_bits),
            vertex.uv.map(f32::to_bits),
            vertex.map_uv.map(|uv| uv.map(f32::to_bits)),
            vertex.tangent.map(f32::to_bits),
            i % 3,
        );
        let index = *lookup.entry(key).or_insert_with(|| {
            // Keep index % 3 equal to the triangle corner for shader barycentrics.
            while unique.len() % 3 != i % 3 {
                unique.push(*vertex);
            }
            let index = unique.len() as u32;
            unique.push(*vertex);
            index
        });
        indices.push(index);
    }
    (unique, indices)
}

#[cfg(test)]
#[test]
fn indexed_geometry_preserves_vertices_and_edge_corners() {
    let a = mesh::Vertex {
        position: [1.0, 2.0, 3.0],
        normal: [0.0, 0.0, 1.0],
        ..Default::default()
    };
    let b = mesh::Vertex {
        position: [2.0, 3.0, 4.0],
        ..a
    };
    let source = [a, b, a, b, a, b, a, b, a];
    let (unique, indices) = indexed_vertices(&source);
    for (corner, (&index, vertex)) in indices.iter().zip(&source).enumerate() {
        assert_eq!(index as usize % 3, corner % 3);
        assert_eq!(unique[index as usize].position, vertex.position);
        assert_eq!(unique[index as usize].normal, vertex.normal);
    }
    assert_eq!(&indices[..3], &indices[6..]);
}

fn vertex_chunk_size(max_buffer_size: u64) -> usize {
    (max_buffer_size.min(64 * 1024 * 1024) / (std::mem::size_of::<mesh::Vertex>() as u64 * 3))
        as usize
        * 3
}

#[cfg(test)]
mod buffer_tests {
    use super::*;
    #[test]
    fn large_mesh_chunks_preserve_triangles_and_device_limits() {
        for limit in [256 * 1024 * 1024, 32 * 1024 * 1024] {
            let count = vertex_chunk_size(limit);
            assert_eq!(count % 3, 0);
            assert!(count * std::mem::size_of::<mesh::Vertex>() <= limit as usize);
            let triangles = 9_994_566usize;
            let chunks = (triangles * 3).div_ceil(count);
            assert!(chunks > 1);
        }
    }
}

#[cfg(test)]
#[test]
fn material_shader_and_all_primitive_pipelines_validate() {
    let instance = wgpu::Instance::default();
    let Some(adapter) =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
    else {
        return;
    };
    let (device, _) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
            .unwrap();
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
    });
    let uniforms = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let textures = GpuState::texture_bind_group_layout(&device);
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        width: 32,
        height: 32,
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 2,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
    };
    for samples in [1, 4] {
        assert_eq!(
            GpuState::create_pipeline(&device, &config, &shader, &uniforms, &textures, samples)
                .len(),
            12
        );
    }
    let error = pollster::block_on(device.pop_error_scope());
    assert!(error.is_none(), "{error:?}");
}

#[cfg(test)]
mod sample_tests;

#[cfg(test)]
#[test]
fn source_indices_preserve_flat_normals_and_edge_corners() {
    let a = mesh::Vertex::default();
    let b = mesh::Vertex {
        normal: [1., 0., 0.],
        ..a
    };
    let vertices = [a, a, a, a, a, a, b, b, b];
    let (unique, indices) = indexed_source_vertices(&vertices, &[0; 9]);
    for (i, index) in indices.iter().enumerate() {
        assert_eq!(
            bytemuck::bytes_of(&unique[*index as usize]),
            bytemuck::bytes_of(&vertices[i])
        );
        assert_eq!(*index as usize % 3, i % 3);
    }
    assert_eq!(indices[0], indices[3]);
    assert_ne!(indices[0], indices[6]);
}

#[cfg(test)]
#[test]
fn uploads_run_off_thread_and_cancel_between_chunks() {
    let instance = wgpu::Instance::default();
    let Some(adapter) =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
    else {
        return;
    };
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
            .unwrap();
    let device = Arc::new(device);
    let queue = Arc::new(queue);
    let layout = Arc::new(GpuState::texture_bind_group_layout(&device));
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let (delivery, received) = mpsc::channel();
    let main_thread = std::thread::current().id();
    let worker = UploadWorker::with_delivery(
        device.clone(),
        queue.clone(),
        layout.clone(),
        move |prepared| {
            delivery
                .send((std::thread::current().id(), prepared))
                .is_ok()
        },
    );
    let revision = worker.latest.fetch_add(1, Ordering::SeqCst) + 1;
    let mut mesh = mesh::Mesh::empty();
    let chunk_size = vertex_chunk_size(device.limits().max_buffer_size).min(196_608);
    mesh.vertices = Arc::new(vec![mesh::Vertex::default(); chunk_size + 3]);
    mesh.textures = Arc::new(vec![mesh::Texture {
        width: 1,
        height: 1,
        pixels: vec![128, 64, 32, 255],
        wrap: [1; 2],
        linear: [true; 2],
    }]);
    mesh.batches.push(mesh::Batch {
        range: 0..mesh.vertices.len(),
        source_indices: None,
        material: Default::default(),
        topology: mesh::Topology::Triangles,
        texture: Some(0),
        // Use the same image as a color map and as linear material data.
        maps: [Some(0), None, None, None],
        double_sided: true,
        blend: false,
    });
    worker
        .send(UploadJob {
            revision,
            mesh: mesh.clone(),
            cached: None,
        })
        .unwrap();
    let (upload_thread, prepared) = received
        .recv_timeout(std::time::Duration::from_secs(30))
        .unwrap();
    assert_ne!(upload_thread, main_thread);
    assert_eq!(prepared.revision, revision);
    assert_eq!(prepared.geometry.len(), 2);
    assert_eq!(
        prepared
            .geometry
            .iter()
            .map(|c| c.count as usize)
            .sum::<usize>(),
        mesh.vertices.len()
    );
    assert_eq!(prepared.textures.len(), 1);
    worker.cancel();
    assert_ne!(prepared.revision, worker.latest.load(Ordering::SeqCst));

    // A queued superseded request must never replace the newer selection.
    let newest = worker.latest.load(Ordering::SeqCst);
    worker
        .send(UploadJob {
            revision,
            mesh: mesh.clone(),
            cached: None,
        })
        .unwrap();
    worker
        .send(UploadJob {
            revision: newest,
            mesh: mesh.clone(),
            cached: None,
        })
        .unwrap();
    let (_, replacement) = received
        .recv_timeout(std::time::Duration::from_secs(30))
        .unwrap();
    assert_eq!(replacement.revision, newest);

    // Cancel after the first chunk has been prepared, before the second starts.
    let checks = std::cell::Cell::new(0);
    let cancelled = GpuState::create_vertex_buffers_cancellable(&device, &mesh, || {
        checks.set(checks.get() + 1);
        checks.get() == 1
    });
    assert!(cancelled.is_none());
    assert_eq!(checks.get(), 2);
    assert!(
        GpuState::create_textures_cancellable(&device, &queue, &layout, &mesh, || false).is_none()
    );

    drop(worker);
    assert_eq!(
        received.try_recv().unwrap_err(),
        mpsc::TryRecvError::Disconnected
    );
    device.poll(wgpu::Maintain::Wait);
    let error = pollster::block_on(device.pop_error_scope());
    assert!(error.is_none(), "GPU validation error: {error:?}");
}

#[cfg(test)]
#[test]
fn large_chunks_skip_deduplication_when_vertices_are_unique() {
    let mut vertices: Vec<_> = (0..12_288)
        .map(|i| mesh::Vertex {
            position: [i as f32, 0., 0.],
            ..Default::default()
        })
        .collect();
    assert!(!should_deduplicate(&vertices));
    // A region with high reuse near the end must still get indexed.
    vertices[11_520..].fill(mesh::Vertex::default());
    assert!(should_deduplicate(&vertices));
}
