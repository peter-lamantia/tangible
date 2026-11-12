use crate::{camera, mesh, theme};
use std::sync::Arc;
use wgpu::util::DeviceExt;
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

struct GeometryChunk {
    vertices: wgpu::Buffer,
    indices: Option<wgpu::Buffer>,
    count: u32,
    topology: usize,
    texture: usize,
    blend: bool,
    center: glam::Vec3,
}

pub(crate) struct GpuState {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub(crate) config: wgpu::SurfaceConfiguration,
    pipeline: Vec<wgpu::RenderPipeline>,
    texture_layout: wgpu::BindGroupLayout,
    textures: Vec<wgpu::BindGroup>,
    animation: Option<Arc<crate::model::gltf::Scene>>,
    animation_start: std::time::Instant,
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
    pub(crate) fn new(window: Arc<Window>, mesh: &mesh::Mesh) -> Result<Self, String> {
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
            },
            None,
        ))
        .map_err(|e| format!("Could not initialize graphics device: {e}"))?;

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

        let texture_layout = Self::texture_bind_group_layout(&device);
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

        let egui_renderer = egui_wgpu::Renderer::new(&device, config.format, None, 1);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            texture_layout,
            textures,
            animation: mesh.animation.clone(),
            animation_start: std::time::Instant::now(),
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
                        buffers: &[wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<mesh::Vertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &[
                                wgpu::VertexAttribute {
                                    offset: 116,
                                    shader_location: 11,
                                    format: wgpu::VertexFormat::Float32x4,
                                },
                                wgpu::VertexAttribute {
                                    offset: 76,
                                    shader_location: 6,
                                    format: wgpu::VertexFormat::Float32x2,
                                },
                                wgpu::VertexAttribute {
                                    offset: 84,
                                    shader_location: 7,
                                    format: wgpu::VertexFormat::Float32x2,
                                },
                                wgpu::VertexAttribute {
                                    offset: 92,
                                    shader_location: 8,
                                    format: wgpu::VertexFormat::Float32x2,
                                },
                                wgpu::VertexAttribute {
                                    offset: 100,
                                    shader_location: 9,
                                    format: wgpu::VertexFormat::Float32x2,
                                },
                                wgpu::VertexAttribute {
                                    offset: 108,
                                    shader_location: 10,
                                    format: wgpu::VertexFormat::Float32x2,
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
                                    shader_location: 4,
                                    format: wgpu::VertexFormat::Float32x4,
                                },
                                wgpu::VertexAttribute {
                                    offset: 64,
                                    shader_location: 5,
                                    format: wgpu::VertexFormat::Float32x3,
                                },
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
                            ],
                        }],
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
                })
            })
            .collect()
    }

    fn create_vertex_buffers(device: &wgpu::Device, mesh: &mesh::Mesh) -> Vec<GeometryChunk> {
        // Bound temporary deduplication memory as well as device buffer sizes.
        let chunk_size = vertex_chunk_size(device.limits().max_buffer_size).min(196_608);
        let batches = if mesh.batches.is_empty() {
            vec![mesh::Batch {
                range: 0..mesh.vertices.len(),
                topology: 0,
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
                    .map(move |vertices| (batch_index, batch, vertices))
            })
            .map(|(batch_index, batch, vertices)| {
                let (unique, indices) = indexed_vertices(vertices);
                let indexed = unique.len() * std::mem::size_of::<mesh::Vertex>()
                    + indices.len() * 4
                    < std::mem::size_of_val(vertices);
                let data = if indexed { unique.as_slice() } else { vertices };
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Model vertices"),
                    contents: bytemuck::cast_slice(data),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let index_buffer = indexed.then(|| {
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Model indices"),
                        contents: bytemuck::cast_slice(&indices),
                        usage: wgpu::BufferUsages::INDEX,
                    })
                });
                GeometryChunk {
                    vertices: buffer,
                    indices: index_buffer,
                    count: vertices.len() as u32,
                    topology: batch.topology as usize
                        + usize::from(!batch.double_sided) * 3
                        + usize::from(batch.blend) * 6,
                    texture: batch_index,
                    blend: batch.blend,
                    center: vertices.iter().fold(glam::Vec3::ZERO, |sum, v| {
                        sum + glam::Vec3::from_array(v.position) / vertices.len() as f32
                    }),
                }
            })
            .collect()
    }

    pub(crate) fn load_mesh(&mut self, mesh: &mesh::Mesh) {
        self.animation = mesh.animation.clone();
        self.animation_start = std::time::Instant::now();
        if Arc::ptr_eq(&self.mesh_storage, &mesh.vertices) {
            return;
        }
        let cached = self
            .geometry_cache
            .iter()
            .position(|(storage, _)| Arc::ptr_eq(storage, &mesh.vertices))
            .and_then(|i| self.geometry_cache.remove(i));
        let previous = std::mem::take(&mut self.vertex_buffers);
        // Retain at most two small meshes; large models release their GPU storage immediately.
        if self.mesh_storage.len() * std::mem::size_of::<mesh::Vertex>() <= 32 * 1024 * 1024 {
            self.geometry_cache
                .push_back((self.mesh_storage.clone(), previous));
        }
        while self.geometry_cache.len() > 2 {
            self.geometry_cache.pop_front();
        }
        self.vertex_buffers = cached
            .map(|(_, buffers)| buffers)
            .unwrap_or_else(|| Self::create_vertex_buffers(&self.device, mesh));
        self.mesh_storage = mesh.vertices.clone();
        self.num_vertices = mesh.vertices.len() as u32;
        self.textures =
            Self::create_textures(&self.device, &self.queue, &self.texture_layout, mesh);
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
        // Upload each source once. Color maps use an sRGB view; data maps use linear views.
        let uploaded: Vec<_> = [&white, &normal]
            .into_iter()
            .chain(mesh.textures.iter())
            .map(|image| {
                let size = wgpu::Extent3d {
                    width: image.width,
                    height: image.height,
                    depth_or_array_layers: 1,
                };
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Material texture"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
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
                (texture, sampler)
            })
            .collect();
        let maps: Vec<[Option<usize>; 5]> = if mesh.batches.is_empty() {
            vec![[None; 5]]
        } else {
            mesh.batches
                .iter()
                .map(|b| [b.texture, b.maps[0], b.maps[1], b.maps[2], b.maps[3]])
                .collect()
        };
        maps.iter()
            .map(|maps| {
                let indices: [usize; 5] = std::array::from_fn(|slot| {
                    maps[slot].map_or(usize::from(slot == 2), |i| i + 2)
                });
                let views: Vec<_> = indices
                    .iter()
                    .enumerate()
                    .map(|(slot, &i)| {
                        uploaded[i].0.create_view(&wgpu::TextureViewDescriptor {
                            format: Some(if slot == 0 || slot == 4 {
                                wgpu::TextureFormat::Rgba8UnormSrgb
                            } else {
                                wgpu::TextureFormat::Rgba8Unorm
                            }),
                            ..Default::default()
                        })
                    })
                    .collect();
                let entries: Vec<_> = (0..10)
                    .map(|binding| wgpu::BindGroupEntry {
                        binding,
                        resource: if binding % 2 == 0 {
                            wgpu::BindingResource::TextureView(&views[binding as usize / 2])
                        } else {
                            wgpu::BindingResource::Sampler(
                                &uploaded[indices[binding as usize / 2]].1,
                            )
                        },
                    })
                    .collect();
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Material maps"),
                    layout,
                    entries: &entries,
                })
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

    pub(crate) fn is_animated(&self) -> bool {
        self.animation.is_some()
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
        if let Some(animation) = &self.animation {
            match animation.evaluate(self.animation_start.elapsed().as_secs_f32()) {
                Ok(mesh) => {
                    self.vertex_buffers = Self::create_vertex_buffers(&self.device, &mesh);
                    self.num_vertices = mesh.vertices.len() as u32;
                }
                Err(error) => {
                    log::error!("Animation failed: {error}");
                    self.animation = None;
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
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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

            self.egui_renderer
                .render(&mut pass, &clipped_primitives, &screen_descriptor);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        for id in &textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
    }
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
            vertex.surface.map(f32::to_bits),
            vertex.emissive.map(f32::to_bits),
            vertex.map_uv.map(|uv| uv.map(f32::to_bits)),
            vertex.map_factors.map(f32::to_bits),
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
