mod camera;
mod model;
mod stl;
mod theme;

use std::sync::Arc;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Event, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{EventLoop, EventLoopBuilder, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Window, WindowBuilder};

use wgpu::util::DeviceExt;

// ── Uniform buffer layout (must match shader) ────────────────────────────────

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
    _pad4: f32,
}

// ── Loaded File Info ─────────────────────────────────────────────────────────

struct FileInfo {
    num_triangles: usize,
    dimensions: glam::Vec3,
    center: glam::Vec3,
    extent: f32,
}

// ── Custom Application Events for Background Tasks ──────────────────────────

#[derive(Debug)]
enum AppEvent {
    LoadingStarted {
        name: String,
    },
    MeshLoaded {
        path: std::path::PathBuf,
        document: model::ModelDocument,
    },
    LoadError {
        path: std::path::PathBuf,
        error: String,
    },
}

// ── GPU state ────────────────────────────────────────────────────────────────

struct GpuState {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    depth_view: wgpu::TextureView,
    msaa_view: Option<wgpu::TextureView>,
    sample_count: u32,
    msaa_supported: bool,
    shader: wgpu::ShaderModule,
    bind_group_layout: wgpu::BindGroupLayout,
    egui_renderer: egui_wgpu::Renderer,
    num_vertices: u32,
}

impl GpuState {
    fn new(window: Arc<Window>, mesh: &stl::Mesh) -> Self {
        let size = window.inner_size();

        // ── Instance + Surface + Adapter + Device ────────────────────────
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone()).unwrap();

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("No suitable GPU adapter found");

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
            },
            None,
        ))
        .expect("Failed to create GPU device");

        // ── Surface config ───────────────────────────────────────────────
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: caps.alpha_modes[0],
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
        let vertex_buffer = Self::create_vertex_buffer(&device, mesh);

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

        let sample_count = 1;
        let pipeline =
            Self::create_pipeline(&device, &config, &shader, &bind_group_layout, sample_count);
        let depth_view = Self::create_depth_texture(&device, &config, sample_count);
        let msaa_view = None;

        let egui_renderer = egui_wgpu::Renderer::new(&device, config.format, None, 1);

        Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            vertex_buffer,
            uniform_buffer,
            bind_group,
            depth_view,
            msaa_view,
            sample_count,
            msaa_supported,
            shader,
            bind_group_layout,
            egui_renderer,
            num_vertices: mesh.vertices.len() as u32,
        }
    }

    fn create_pipeline(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        shader: &wgpu::ShaderModule,
        bind_group_layout: &wgpu::BindGroupLayout,
        sample_count: u32,
    ) -> wgpu::RenderPipeline {
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<stl::Vertex>() as u64,
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
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // show both faces
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
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
    }

    fn create_vertex_buffer(device: &wgpu::Device, mesh: &stl::Mesh) -> wgpu::Buffer {
        if mesh.vertices.is_empty() {
            let dummy = stl::Vertex {
                position: [0.0; 3],
                normal: [0.0; 3],
            };
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Vertex Buffer (Dummy)"),
                contents: bytemuck::cast_slice(&[dummy]),
                usage: wgpu::BufferUsages::VERTEX,
            })
        } else {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Vertex Buffer"),
                contents: bytemuck::cast_slice(&mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            })
        }
    }

    fn load_mesh(&mut self, mesh: &stl::Mesh) {
        self.vertex_buffer = Self::create_vertex_buffer(&self.device, mesh);
        self.num_vertices = mesh.vertices.len() as u32;
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

    fn set_antialiasing(&mut self, enabled: bool) {
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
            sample_count,
        );
        self.depth_view = Self::create_depth_texture(&self.device, &self.config, sample_count);
        self.msaa_view = self.create_msaa_view();
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
            self.depth_view =
                Self::create_depth_texture(&self.device, &self.config, self.sample_count);
            self.msaa_view = self.create_msaa_view();
        }
    }

    fn render(
        &mut self,
        cam: &camera::Camera,
        egui_ctx: &egui::Context,
        egui_shapes: Vec<egui::epaint::ClippedShape>,
        textures_delta: &egui::TexturesDelta,
        pixels_per_point: f32,
        scene_left: u32,
        is_dark: bool,
    ) {
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
            _pad4: 0.0,
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
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.draw(0..self.num_vertices, 0..1);
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

// ── Helpers ──────────────────────────────────────────────────────────────────

fn format_number(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i) % 3 == 0 {
            result.push(',');
        }
        result.push(*c);
    }
    result
}

fn plate_preview(mesh: &stl::Mesh) -> egui::ColorImage {
    const SIZE: usize = 320;
    let background = egui::Color32::from_rgb(32, 39, 49);
    let mut image = egui::ColorImage::new([SIZE, SIZE], background);
    if mesh.vertices.is_empty() {
        return image;
    }
    let mut depth = vec![f32::INFINITY; SIZE * SIZE];
    let mut camera = camera::Camera::new(mesh.center(), 5.0);
    camera.aspect = 1.0;
    camera.frame_target(mesh.center(), mesh.extent());
    let mvp = camera.projection_matrix() * camera.view_matrix();
    let light = glam::Vec3::new(0.5, 1.0, 0.8).normalize();
    let eye = camera.eye();
    for triangle in mesh.vertices.chunks_exact(3) {
        let mut screen = [[0.0f32; 3]; 3];
        let mut valid = true;
        for (i, vertex) in triangle.iter().enumerate() {
            let clip = mvp * glam::Vec3::from(vertex.position).extend(1.0);
            if clip.w <= 0.0 {
                valid = false;
                break;
            }
            let ndc = clip.truncate() / clip.w;
            screen[i] = [
                (ndc.x * 0.5 + 0.5) * SIZE as f32,
                (0.5 - ndc.y * 0.5) * SIZE as f32,
                ndc.z,
            ];
        }
        if !valid {
            continue;
        }
        let edge = |a: [f32; 3], b: [f32; 3], x: f32, y: f32| {
            (x - a[0]) * (b[1] - a[1]) - (y - a[1]) * (b[0] - a[0])
        };
        let area = edge(screen[0], screen[1], screen[2][0], screen[2][1]);
        if area.abs() < 1e-8 {
            continue;
        }
        let x_min = screen
            .iter()
            .map(|p| p[0])
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as usize;
        let x_max = screen
            .iter()
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .min((SIZE - 1) as f32) as usize;
        let y_min = screen
            .iter()
            .map(|p| p[1])
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as usize;
        let y_max = screen
            .iter()
            .map(|p| p[1])
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .min((SIZE - 1) as f32) as usize;
        if x_min > x_max || y_min > y_max {
            continue;
        }
        let mut normal = glam::Vec3::from(triangle[0].normal).normalize_or_zero();
        if normal.dot(light) < 0.0 {
            normal = -normal;
        }
        let diffuse = normal.dot(light).max(0.0);
        let center = (glam::Vec3::from(triangle[0].position)
            + glam::Vec3::from(triangle[1].position)
            + glam::Vec3::from(triangle[2].position))
            / 3.0;
        let half_dir = (light + (eye - center).normalize_or_zero()).normalize_or_zero();
        let specular = normal.dot(half_dir).max(0.0).powf(32.0) * 0.3;
        let intensity = 0.18 + diffuse + specular;
        let color = egui::Color32::from_rgb(
            (0.7 * intensity * 255.0).clamp(0.0, 255.0) as u8,
            (0.75 * intensity * 255.0).clamp(0.0, 255.0) as u8,
            (0.8 * intensity * 255.0).clamp(0.0, 255.0) as u8,
        );
        for y in y_min..=y_max {
            for x in x_min..=x_max {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let w0 = edge(screen[1], screen[2], px, py) / area;
                let w1 = edge(screen[2], screen[0], px, py) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * screen[0][2] + w1 * screen[1][2] + w2 * screen[2][2];
                let index = y * SIZE + x;
                if z < depth[index] {
                    depth[index] = z;
                    image[(x, y)] = color;
                }
            }
        }
    }
    image
}

fn plate_tile(
    ui: &mut egui::Ui,
    label: &str,
    texture: &egui::TextureHandle,
    selected: bool,
) -> bool {
    let width = (ui.available_width() - 8.0).clamp(120.0, 360.0);
    let image_size = width - 16.0;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, image_size + 44.0), egui::Sense::click());
    let fill = if selected {
        ui.visuals().selection.bg_fill
    } else {
        ui.visuals().faint_bg_color
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    let image_rect = egui::Rect::from_min_size(
        rect.min + egui::vec2(8.0, 8.0),
        egui::vec2(image_size, image_size),
    );
    ui.painter().image(
        texture.id(),
        image_rect,
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
    ui.painter().text(
        egui::pos2(rect.center().x, rect.bottom() - 20.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::TextStyle::Body.resolve(ui.style()),
        ui.visuals().text_color(),
    );
    ui.add_space(8.0);
    response.clicked()
}

#[cfg(test)]
#[test]
fn plate_preview_renders_geometry() {
    let mesh = stl::Mesh {
        vertices: vec![
            stl::Vertex {
                position: [-1.0, -1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
            },
            stl::Vertex {
                position: [1.0, -1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
            },
            stl::Vertex {
                position: [0.0, 1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
            },
        ],
        bounds_min: glam::Vec3::new(-1.0, -1.0, 0.0),
        bounds_max: glam::Vec3::new(1.0, 1.0, 0.0),
    };
    let preview = plate_preview(&mesh);
    assert_eq!(preview.size, [320, 320]);
    assert!(preview
        .pixels
        .iter()
        .any(|pixel| *pixel != egui::Color32::from_rgb(32, 39, 49)));
}

/// Spawns the native file picker on a background thread so the window event loop never freezes.
fn spawn_open_dialog(proxy: EventLoopProxy<AppEvent>) {
    std::thread::spawn(move || {
        let dialog = rfd::FileDialog::new()
            .add_filter("3D Models (*.stl, *.obj, *.3mf)", &["stl", "obj", "3mf"])
            .set_title("Open 3D Model");

        if let Some(path) = dialog.pick_file() {
            spawn_file_load(path, proxy);
        }
    });
}

/// Reads and parses an STL file on a background thread, notifying the main loop via proxy.
fn spawn_file_load(path: std::path::PathBuf, proxy: EventLoopProxy<AppEvent>) {
    let file_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let _ = proxy.send_event(AppEvent::LoadingStarted { name: file_name });

    std::thread::spawn(move || match model::load_document(&path) {
        Ok(document) => {
            let _ = proxy.send_event(AppEvent::MeshLoaded { path, document });
        }
        Err(e) => {
            let _ = proxy.send_event(AppEvent::LoadError {
                path,
                error: e.to_string(),
            });
        }
    });
}

// ── Entry point ──────────────────────────────────────────────────────────────

fn main() {
    env_logger::init();

    let args: Vec<String> = std::env::args().collect();
    let initial_path = if args.len() >= 2 {
        Some(std::path::PathBuf::from(&args[1]))
    } else {
        None
    };

    let event_loop: EventLoop<AppEvent> = EventLoopBuilder::<AppEvent>::with_user_event()
        .build()
        .unwrap();
    let proxy = event_loop.create_proxy();

    let window = Arc::new(
        WindowBuilder::new()
            .with_title("Tangible")
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
            .build(&event_loop)
            .expect("Failed to create window"),
    );

    let egui_ctx = egui::Context::default();
    theme::setup_fonts(&egui_ctx);
    let cube_image = egui::ColorImage::from_rgba_unmultiplied(
        [256, 256],
        include_bytes!("../assets/impossible_cube_fuchsia.rgba"),
    );
    let cube_texture =
        egui_ctx.load_texture("impossible_cube", cube_image, egui::TextureOptions::LINEAR);

    let system_theme = window.theme().unwrap_or(winit::window::Theme::Dark);
    let mut system_is_dark = matches!(system_theme, winit::window::Theme::Dark);
    let mut theme_pref = theme::ThemePreference::System;
    let mut is_dark = theme_pref.is_dark(system_is_dark);
    theme::apply_theme(&egui_ctx, is_dark);

    let mut egui_state = egui_winit::State::new(
        egui_ctx.clone(),
        egui::ViewportId::ROOT,
        &window,
        Some(window.scale_factor() as f32),
        None,
    );

    let mut file_info: Option<FileInfo> = None;
    let mut loading_name: Option<String> = None;
    let mut error_message: Option<String> = None;
    let mut camera = camera::Camera::new(glam::Vec3::ZERO, 5.0);

    let size = window.inner_size();
    camera.aspect = size.width as f32 / size.height.max(1) as f32;

    let initial_mesh = stl::Mesh::empty();
    let mut gpu = GpuState::new(window.clone(), &initial_mesh);
    gpu.set_antialiasing(true);
    let mut document: Option<model::ModelDocument> = None;
    let mut plate_previews: Vec<egui::TextureHandle> = Vec::new();
    let mut all_preview: Option<egui::TextureHandle> = None;
    let mut selected_plate: Option<usize> = None;
    let mut plates_collapsed = false;

    // Load initial file in background if specified via CLI
    if let Some(path) = initial_path {
        spawn_file_load(path, proxy.clone());
    }

    // ── Interaction state ────────────────────────────────────────────────
    let mut left_pressed = false;
    let mut right_pressed = false;
    let mut last_cursor: Option<(f64, f64)> = None;
    let mut modifiers = ModifiersState::default();

    let mut should_open_file = false;
    let mut should_reset_camera = false;
    let mut should_clear_error = false;
    let mut should_update_theme = false;
    let mut requested_plate: Option<Option<usize>> = None;

    // ── Run event loop ───────────────────────────────────────────────────
    event_loop
        .run(move |event, elwt| match event {
            // ── Background Task Events ───────────────────────────────────────
            Event::UserEvent(app_event) => match app_event {
                AppEvent::LoadingStarted { name } => {
                    loading_name = Some(name);
                    error_message = None;
                    window.request_redraw();
                }
                AppEvent::MeshLoaded { path, document: loaded } => {
                    let mesh = &loaded.mesh;
                    let num_triangles = mesh.vertices.len() / 3;
                    let center = mesh.center();
                    let extent = mesh.extent();
                    let dims = mesh.dimensions();
                    let file_name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    camera.frame_target(center, extent);
                    gpu.load_mesh(mesh);
                    window.set_title(&format!("Tangible — {file_name}"));

                    file_info = Some(FileInfo {
                        num_triangles,
                        dimensions: dims,
                        center,
                        extent,
                    });
                    all_preview = Some(egui_ctx.load_texture("all-plates-preview", plate_preview(&loaded.mesh), egui::TextureOptions::LINEAR));
                    plate_previews = loaded.plates.iter().enumerate().map(|(i, plate)| {
                        egui_ctx.load_texture(format!("plate-preview-{i}"), plate_preview(&plate.mesh), egui::TextureOptions::LINEAR)
                    }).collect();
                    selected_plate = None;
                    document = Some(loaded);
                    loading_name = None;
                    error_message = None;
                    window.request_redraw();
                }
                AppEvent::LoadError { path, error } => {
                    loading_name = None;
                    error_message =
                        Some(format!("Failed to load \"{}\": {error}", path.display()));
                    window.request_redraw();
                }
            },

            // ── Window Events ────────────────────────────────────────────────
            Event::WindowEvent { event, .. } => {
                let response = egui_state.on_window_event(&window, &event);
                if response.repaint {
                    window.request_redraw();
                }

                if response.consumed {
                    if let WindowEvent::MouseInput {
                        state: ElementState::Released,
                        ..
                    } = &event
                    {
                        left_pressed = false;
                        right_pressed = false;
                    }
                    return;
                }

                match event {
                    WindowEvent::CloseRequested => elwt.exit(),

                    WindowEvent::ThemeChanged(theme) => {
                        system_is_dark = matches!(theme, winit::window::Theme::Dark);
                        let new_is_dark = theme_pref.is_dark(system_is_dark);
                        if new_is_dark != is_dark {
                            is_dark = new_is_dark;
                            theme::apply_theme(&egui_ctx, is_dark);
                        }
                        window.request_redraw();
                    }

                    WindowEvent::Resized(size) => {
                        gpu.resize(size.width, size.height);
                        camera.aspect = size.width as f32 / size.height.max(1) as f32;
                    }

                    WindowEvent::DroppedFile(path) => {
                        spawn_file_load(path, proxy.clone());
                    }

                    WindowEvent::ModifiersChanged(new_modifiers) => {
                        modifiers = new_modifiers.state();
                    }

                    WindowEvent::KeyboardInput {
                        event: key_event, ..
                    } => {
                        if key_event.state == ElementState::Pressed && !key_event.repeat {
                            let is_ctrl_or_cmd =
                                modifiers.control_key() || modifiers.super_key();
                            let is_o = match &key_event.logical_key {
                                Key::Character(s) => s.eq_ignore_ascii_case("o"),
                                _ => key_event.physical_key == PhysicalKey::Code(KeyCode::KeyO),
                            };

                            if is_o && (is_ctrl_or_cmd || !egui_ctx.wants_keyboard_input()) {
                                spawn_open_dialog(proxy.clone());
                            }
                        }
                    }

                    WindowEvent::MouseInput { state, button, .. } => {
                        let pressed = state == ElementState::Pressed;
                        if pressed && egui_ctx.wants_pointer_input() {
                            return;
                        }
                        match button {
                            MouseButton::Left => left_pressed = pressed,
                            MouseButton::Right | MouseButton::Middle => {
                                right_pressed = pressed
                            }
                            _ => {}
                        }
                    }

                    WindowEvent::CursorMoved { position, .. } => {
                        if let Some((lx, ly)) = last_cursor {
                            let dx = position.x - lx;
                            let dy = position.y - ly;

                            if left_pressed {
                                camera.orbit(dx as f32, dy as f32);
                                window.request_redraw();
                            }
                            if right_pressed {
                                camera.pan(dx as f32, dy as f32);
                                window.request_redraw();
                            }
                        }
                        last_cursor = Some((position.x, position.y));
                    }

                    WindowEvent::MouseWheel { delta, .. } => {
                        if egui_ctx.wants_pointer_input() {
                            return;
                        }
                        let scroll = match delta {
                            MouseScrollDelta::LineDelta(_, y) => y,
                            MouseScrollDelta::PixelDelta(p) => p.y as f32 / 50.0,
                        };
                        camera.zoom(scroll);
                        window.request_redraw();
                    }

                    WindowEvent::Focused(false) => {
                        left_pressed = false;
                        right_pressed = false;
                    }

                    WindowEvent::RedrawRequested => {
                        let raw_input = egui_state.take_egui_input(&window);
                        let mut scene_left_points = 0.0;
                        let full_output = egui_ctx.run(raw_input, |ctx| {
                            // ── Top Bar ──────────────────────────────────────────
                            egui::TopBottomPanel::top("top_panel")
                                .frame(
                                    egui::Frame::none()
                                        .fill(ctx.style().visuals.panel_fill)
                                        .inner_margin(egui::Margin::symmetric(12.0, 7.0)),
                                )
                                .show(ctx, |ui| {
                                    ui.horizontal(|ui| {
                                        if theme::radix_button(
                                            ui,
                                            Some(theme::Icon::Folder),
                                            "Open Model",
                                        )
                                        .on_hover_text("Open an STL, OBJ, or 3MF file from disk (Ctrl+O / O)")
                                        .clicked()
                                        {
                                            should_open_file = true;
                                        }

                                        if let Some(loading) = &loading_name {
                                            ui.add_space(6.0);
                                            ui.spinner();
                                            ui.label(
                                                egui::RichText::new(format!("Loading {loading}..."))
                                                    .color(if is_dark {
                                                        egui::Color32::from_rgb(147, 197, 253)
                                                    } else {
                                                        egui::Color32::from_rgb(37, 99, 235)
                                                    }),
                                            );
                                        }

                                        // Right-aligned: display settings
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if gpu.msaa_supported {
                                                    let mut enabled = gpu.sample_count > 1;
                                                    if ui.checkbox(&mut enabled, "4× AA").on_hover_text("Smooth 3D model edges with 4× MSAA").changed() {
                                                        gpu.set_antialiasing(enabled);
                                                    }
                                                    ui.add_space(10.0);
                                                }
                                                if theme::theme_segmented_control(
                                                    ui,
                                                    &mut theme_pref,
                                                ) {
                                                    should_update_theme = true;
                                                }

                                            },
                                        );
                                    });
                                });

                            if let Some(doc) = &document {
                                if !doc.plates.is_empty() {
                                    if plates_collapsed {
                                        let panel = egui::SidePanel::left("plates_panel_collapsed")
                                            .resizable(false)
                                            .exact_width(24.0)
                                            .frame(egui::Frame::none()
                                                .fill(ctx.style().visuals.panel_fill)
                                                .inner_margin(0.0))
                                            .show(ctx, |ui| {
                                                let rect = ui.max_rect();
                                                let response = ui.interact(
                                                    rect,
                                                    ui.id().with("expand_plates"),
                                                    egui::Sense::click(),
                                                ).on_hover_cursor(egui::CursorIcon::PointingHand)
                                                 .on_hover_text("Expand plates panel");
                                                ui.painter().text(
                                                    egui::pos2(rect.center().x, rect.top() + 20.0),
                                                    egui::Align2::CENTER_CENTER,
                                                    "›",
                                                    egui::TextStyle::Heading.resolve(ui.style()),
                                                    ui.visuals().text_color(),
                                                );
                                                if response.clicked() { plates_collapsed = false; }
                                            });
                                        scene_left_points = panel.response.rect.right();
                                    } else {
                                        let panel = egui::SidePanel::left("plates_panel_open")
                                            .resizable(true)
                                            .default_width(176.0)
                                            .width_range(160.0..=400.0)
                                            .show(ctx, |ui| {
                                                ui.horizontal(|ui| {
                                                    ui.heading("Plates");
                                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                        if ui.button("◀").on_hover_text("Minimize plates panel").clicked() {
                                                            plates_collapsed = true;
                                                        }
                                                    });
                                                });
                                                ui.separator();
                                                egui::ScrollArea::vertical().show(ui, |ui| {
                                                    if plate_tile(ui, "All plates", all_preview.as_ref().unwrap(), selected_plate.is_none()) {
                                                        requested_plate = Some(None);
                                                    }
                                                    for (index, plate) in doc.plates.iter().enumerate() {
                                                        if plate_tile(ui, &plate.name, &plate_previews[index], selected_plate == Some(index)) {
                                                            requested_plate = Some(Some(index));
                                                        }
                                                    }
                                                });
                                            });
                                        scene_left_points = panel.response.rect.right();
                                    }
                                }
                            }

                            // ── Bottom Status Bar ────────────────────────────────
                            let bottom_panel = egui::TopBottomPanel::bottom("bottom_panel")
                                .frame(
                                    egui::Frame::none()
                                        .fill(ctx.style().visuals.panel_fill)
                                        .inner_margin(egui::Margin::symmetric(12.0, 5.0)),
                                )
                                .show(ctx, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(
                                                "Left Drag: Orbit  •  Right Drag: Pan  •  Scroll: Zoom",
                                            )
                                            .small()
                                            .color(ui.visuals().widgets.noninteractive.text_color()),
                                        );

                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if let Some(info) = &file_info {
                                                    let color = ui.visuals().widgets.noninteractive.text_color();
                                                    ui.label(egui::RichText::new(format!(
                                                        "{:.1} × {:.1} × {:.1}",
                                                        info.dimensions.x, info.dimensions.y, info.dimensions.z
                                                    )).small().color(color));
                                                    ui.add_space(16.0);
                                                    ui.label(egui::RichText::new(format!(
                                                        "{} triangles", format_number(info.num_triangles)
                                                    )).small().color(color));
                                                }
                                                if let Some(err) = &error_message {
                                                    ui.add_space(12.0);
                                                    if theme::radix_button(ui, Some(theme::Icon::Close), "")
                                                        .on_hover_text("Dismiss error").clicked() {
                                                        should_clear_error = true;
                                                    }
                                                    ui.colored_label(
                                                        if is_dark { egui::Color32::from_rgb(248, 113, 113) }
                                                        else { egui::Color32::from_rgb(220, 38, 38) },
                                                        format!("⚠ {err}"),
                                                    );
                                                }
                                            },
                                        );
                                    });
                                });

                            if file_info.is_some() {
                                egui::Area::new(egui::Id::new("floating_reset_view"))
                                    .order(egui::Order::Foreground)
                                    .anchor(
                                        egui::Align2::RIGHT_BOTTOM,
                                        egui::vec2(-16.0, -bottom_panel.response.rect.height() - 16.0),
                                    )
                                    .show(ctx, |ui| {
                                        let button = egui::Button::new(
                                            egui::RichText::new("↺  Reset view")
                                                .color(egui::Color32::from_rgb(35, 43, 54)),
                                        )
                                        .fill(egui::Color32::from_rgba_unmultiplied(248, 250, 252, 225))
                                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(180)))
                                        .rounding(egui::Rounding::same(6.0));
                                        if ui.add(button).on_hover_text("Reset camera to center of model").clicked() {
                                            should_reset_camera = true;
                                        }
                                    });
                            }

                            // ── Welcome Screen (when no model loaded) ────────────
                            if file_info.is_none() {
                                egui::CentralPanel::default()
                                    .frame(egui::Frame::none())
                                    .show(ctx, |ui| {
                                        let available = ui.available_rect_before_wrap();
                                        let card_w = 380.0_f32;
                                        let card_inner = 32.0_f32;

                                        let card_bg = if is_dark {
                                            egui::Color32::from_rgba_unmultiplied(20, 22, 27, 240)
                                        } else {
                                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 245)
                                        };
                                        let card_stroke = egui::Stroke::new(
                                            1.0,
                                            if is_dark {
                                                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 24)
                                            } else {
                                                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 22)
                                            },
                                        );

                                        // Measure card height by running a dry layout pass
                                        let card_height = {
                                            let icon_h = 72.0_f32;
                                            let heading_h = 24.0_f32;
                                            let subtitle_h = 15.0_f32;
                                            let btn_h = 28.0_f32;
                                            let hint_h = 15.0_f32;
                                            let badge_h = 23.0_f32;
                                            let spacing = 8.0 + 4.0 + 20.0 + 14.0 + 8.0;
                                            icon_h + heading_h + subtitle_h + btn_h + hint_h + badge_h + spacing + card_inner * 2.0
                                        };

                                        // Center the card in the available area
                                        let card_x = available.center().x - card_w * 0.5 - card_inner;
                                        let card_y = (available.center().y - card_height * 0.5).max(available.top() + 16.0);

                                        ui.allocate_ui_at_rect(
                                            egui::Rect::from_min_size(
                                                egui::pos2(card_x, card_y),
                                                egui::vec2(card_w + card_inner * 2.0, card_height),
                                            ),
                                            |ui| {
                                                egui::Frame::none()
                                                    .fill(card_bg)
                                                    .stroke(card_stroke)
                                                    .rounding(egui::Rounding::same(12.0))
                                                    .shadow(ui.visuals().window_shadow)
                                                    .inner_margin(card_inner)
                                                    .show(ui, |ui| {
                                                        ui.set_min_width(card_w);
                                                        ui.set_max_width(card_w);
                                                        ui.vertical_centered(|ui| {
                                                            ui.add(
                                                                egui::Image::new((cube_texture.id(), egui::vec2(72.0, 72.0)))
                                                                    .fit_to_exact_size(egui::vec2(72.0, 72.0)),
                                                            );
                                                            ui.add_space(8.0);

                                                            ui.heading("Tangible");
                                                            ui.add_space(4.0);
                                                            ui.label(
                                                                egui::RichText::new(
                                                                    "Inspect STL, OBJ, and 3MF models",
                                                                )
                                                                .small()
                                                                .color(ui.visuals().widgets.noninteractive.text_color()),
                                                            );
                                                            ui.add_space(20.0);

                                                            if let Some(loading) = &loading_name {
                                                                ui.spinner();
                                                                ui.add_space(8.0);
                                                                ui.label(format!("Loading {loading}..."));
                                                                ui.label(
                                                                    egui::RichText::new(
                                                                        "Parsing triangles in background...",
                                                                    )
                                                                    .small()
                                                                    .weak(),
                                                                );
                                                            } else {
                                                                if theme::radix_button(
                                                                    ui,
                                                                    Some(theme::Icon::Folder),
                                                                    "Open 3D Model",
                                                                )
                                                                .on_hover_text("Open file dialog (Ctrl+O / O)")
                                                                .clicked()
                                                                {
                                                                    should_open_file = true;
                                                                }

                                                                ui.add_space(14.0);
                                                                ui.label(
                                                                    egui::RichText::new(
                                                                        "or drag & drop a .stl, .obj, or .3mf file anywhere",
                                                                    )
                                                                    .small()
                                                                    .weak(),
                                                                );
                                                                ui.add_space(8.0);
                                                                theme::radix_badge(
                                                                    ui,
                                                                    None,
                                                                    "Shortcut: Ctrl+O / O",
                                                                );
                                                            }
                                                        });
                                                    });
                                            },
                                        );
                                    });
                            }
                        });

                        egui_state.handle_platform_output(&window, full_output.platform_output);

                        if let Some(selection) = requested_plate.take() {
                            selected_plate = selection;
                            if let (Some(doc), Some(info)) = (&document, &mut file_info) {
                                let mesh = selection.map(|index| &doc.plates[index].mesh).unwrap_or(&doc.mesh);
                                gpu.load_mesh(mesh);
                                info.num_triangles = mesh.vertices.len() / 3;
                                info.dimensions = mesh.dimensions();
                                info.center = mesh.center();
                                info.extent = mesh.extent();
                                camera.frame_target(info.center, info.extent);
                            }
                            window.request_redraw();
                        }

                        if should_update_theme {
                            should_update_theme = false;
                            let new_is_dark = theme_pref.is_dark(system_is_dark);
                            if new_is_dark != is_dark {
                                is_dark = new_is_dark;
                                theme::apply_theme(&egui_ctx, is_dark);
                            }
                            window.request_redraw();
                        }

                        let scene_left = (scene_left_points * full_output.pixels_per_point).round() as u32;
                        camera.aspect = (gpu.config.width.saturating_sub(scene_left).max(1)) as f32 / gpu.config.height.max(1) as f32;
                        gpu.render(
                            &camera,
                            &egui_ctx,
                            full_output.shapes,
                            &full_output.textures_delta,
                            full_output.pixels_per_point,
                            scene_left,
                            is_dark,
                        );

                        if should_open_file {
                            should_open_file = false;
                            spawn_open_dialog(proxy.clone());
                        }

                        if should_reset_camera {
                            should_reset_camera = false;
                            if let Some(info) = &file_info {
                                camera.frame_target(info.center, info.extent);
                            }
                            window.request_redraw();
                        }

                        if should_clear_error {
                            should_clear_error = false;
                            error_message = None;
                            window.request_redraw();
                        }
                    }

                    _ => {}
                }
            }

            Event::AboutToWait => {
                if egui_ctx.has_requested_repaint() {
                    window.request_redraw();
                }
            }

            _ => {}
        })
        .unwrap();
}
