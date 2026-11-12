//! Opt-in integration coverage using a local Khronos sample checkout and real GPU readback.
use super::*;
use std::path::Path;

const MODELS: [&str; 20] = [
    "BoxTextured/glTF/BoxTextured.gltf",
    "Duck/glTF-Binary/Duck.glb",
    "Avocado/glTF/Avocado.gltf",
    "DamagedHelmet/glTF-Binary/DamagedHelmet.glb",
    "BoxVertexColors/glTF/BoxVertexColors.gltf",
    "VertexColorTest/glTF/VertexColorTest.gltf",
    "BoxTexturedNonPowerOfTwo/glTF/BoxTexturedNonPowerOfTwo.gltf",
    "TextureSettingsTest/glTF/TextureSettingsTest.gltf",
    "TextureTransformTest/glTF/TextureTransformTest.gltf",
    "MultiUVTest/glTF/MultiUVTest.gltf",
    "UnlitTest/glTF/UnlitTest.gltf",
    "AlphaBlendModeTest/glTF/AlphaBlendModeTest.gltf",
    "AnimatedColorsCube/glTF/AnimatedColorsCube.gltf",
    "AnimatedMorphCube/glTF/AnimatedMorphCube.gltf",
    "SimpleSkin/glTF/SimpleSkin.gltf",
    "RiggedSimple/glTF/RiggedSimple.gltf",
    "Fox/glTF/Fox.gltf",
    "InterpolationTest/glTF/InterpolationTest.gltf",
    "MorphPrimitivesTest/glTF/MorphPrimitivesTest.gltf",
    "PrimitiveModeNormalsTest/glTF/PrimitiveModeNormalsTest.gltf",
];

const MORE_MODELS: [&str; 20] = [
    "CesiumMan/glTF/CesiumMan.gltf",
    "BrainStem/glTF/BrainStem.gltf",
    "AnimatedTriangle/glTF/AnimatedTriangle.gltf",
    "RiggedFigure/glTF/RiggedFigure.gltf",
    "MorphStressTest/glTF/MorphStressTest.gltf",
    "RecursiveSkeletons/glTF/RecursiveSkeletons.gltf",
    "NegativeScaleTest/glTF/NegativeScaleTest.gltf",
    "NormalTangentTest/glTF/NormalTangentTest.gltf",
    "NormalTangentMirrorTest/glTF/NormalTangentMirrorTest.gltf",
    "TextureCoordinateTest/glTF/TextureCoordinateTest.gltf",
    "TextureLinearInterpolationTest/glTF/TextureLinearInterpolationTest.gltf",
    "TextureEncodingTest/glTF/TextureEncodingTest.gltf",
    "BoomBox/glTF-Binary/BoomBox.glb",
    "WaterBottle/glTF/WaterBottle.gltf",
    "OrientationTest/glTF/OrientationTest.gltf",
    "Box With Spaces/glTF/Box With Spaces.gltf",
    "Unicode❤♻Test/glTF/Unicode❤♻Test.gltf",
    "SimpleSparseAccessor/glTF/SimpleSparseAccessor.gltf",
    "EnvironmentTest/glTF/EnvironmentTest.gltf",
    "TwoSidedPlane/glTF/TwoSidedPlane.gltf",
];

#[test]
#[ignore = "set TANGIBLE_GLTF_SAMPLES to a local Khronos Models directory; requires a GPU"]
fn render_twenty_khronos_models() {
    run_models(&MODELS, "batch-1");
}

#[test]
#[ignore = "set TANGIBLE_GLTF_SAMPLES to a local Khronos Models directory; requires a GPU"]
fn render_twenty_more_khronos_models() {
    run_models(&MORE_MODELS, "batch-2");
}

fn run_models(models: &[&str], batch: &str) {
    let root = std::env::var("TANGIBLE_GLTF_SAMPLES").expect("set TANGIBLE_GLTF_SAMPLES");
    let output = std::env::var("TANGIBLE_GLTF_OUTPUT")
        .unwrap_or_else(|_| "/tmp/tangible-gltf-samples".into());
    let output = Path::new(&output).join(batch);
    std::fs::create_dir_all(&output).unwrap();
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("GPU adapter");
    eprintln!("GPU: {:?}", adapter.get_info());
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
            .unwrap();
    let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
    let texture_layout = GpuState::texture_bind_group_layout(&device);
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        width: 512,
        height: 384,
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 2,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
    };
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(include_str!("../shader.wgsl").into()),
    });
    let pipelines = GpuState::create_pipeline(
        &device,
        &config,
        &shader,
        &uniform_layout,
        &texture_layout,
        1,
    );
    let mut failures = Vec::new();
    let mut report = String::from("model\tvertices\ttextures\tanimated\tresult\n");
    for &relative in models {
        let name = relative.split('/').next().unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || -> Result<_, Box<dyn std::error::Error>> {
                let mesh = crate::model::load_model(Path::new(&root).join(relative))?;
                assert!(!mesh.vertices.is_empty());
                if name == "NormalTangentMirrorTest" {
                    assert!(mesh.vertices.iter().any(|v| v.tangent[3] == -1.));
                    assert!(mesh.vertices.iter().any(|v| v.tangent[3] == 1.));
                }
                if name == "NormalTangentTest" {
                    assert!(mesh.vertices.iter().all(|v| v.tangent == [0.; 4]));
                }
                let mut camera = camera::Camera::new(mesh.center(), 1.);
                camera.aspect = config.width as f32 / config.height as f32;
                camera.frame_target(mesh.center(), mesh.extent());
                let mut changed = false;
                for (frame, seconds) in [0., 0.5, 1., 2.].into_iter().enumerate() {
                    let animated = mesh
                        .animation
                        .as_ref()
                        .map(|s| s.evaluate(seconds))
                        .transpose()?;
                    let current = animated.as_ref().unwrap_or(&mesh);
                    assert!(
                        current.vertices.iter().all(|v| v
                            .position
                            .iter()
                            .chain(&v.normal)
                            .chain(&v.color)
                            .chain(&v.tangent)
                            .all(|x| x.is_finite())),
                        "non-finite animated geometry"
                    );
                    changed |= current
                        .vertices
                        .iter()
                        .zip(mesh.vertices.iter())
                        .any(|(a, b)| a.position != b.position || a.color != b.color);
                    if frame < 3 {
                        device.push_error_scope(wgpu::ErrorFilter::Validation);
                        render(
                            &device,
                            &queue,
                            &config,
                            &pipelines,
                            &uniform_layout,
                            &texture_layout,
                            current,
                            &camera,
                            &Path::new(&output).join(format!("{name}-{frame}.ppm")),
                        );
                        assert!(
                            pollster::block_on(device.pop_error_scope()).is_none(),
                            "GPU validation error"
                        );
                    }
                }
                assert!(
                    mesh.animation.is_none() || changed,
                    "animation did not change geometry or colors at any sampled time"
                );
                Ok((
                    mesh.vertices.len(),
                    mesh.textures.len(),
                    mesh.animation.is_some(),
                ))
            },
        ));
        match result {
            Ok(Ok((vertices, textures, animated))) => {
                eprintln!(
                    "PASS {name}: {vertices} vertices, {textures} textures, animated={animated}"
                );
                report.push_str(&format!(
                    "{relative}\t{vertices}\t{textures}\t{animated}\tPASS\n"
                ));
            }
            other => {
                let error = match other {
                    Ok(Err(e)) => e.to_string(),
                    Err(e) => e
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| e.downcast_ref::<&str>().map(|v| v.to_string()))
                        .unwrap_or_else(|| "panic".into()),
                    _ => unreachable!(),
                };
                eprintln!("FAIL {name}: {error}");
                report.push_str(&format!("{relative}\t\t\t\t{error}\n"));
                failures.push((name, error));
            }
        }
    }
    std::fs::write(Path::new(&output).join("results.tsv"), report).unwrap();
    assert!(failures.is_empty(), "{failures:#?}");
}

#[allow(clippy::too_many_arguments)]
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    config: &wgpu::SurfaceConfiguration,
    pipelines: &[wgpu::RenderPipeline],
    uniform_layout: &wgpu::BindGroupLayout,
    texture_layout: &wgpu::BindGroupLayout,
    mesh: &mesh::Mesh,
    camera: &camera::Camera,
    output: &Path,
) {
    let chunks = GpuState::create_vertex_buffers(device, mesh);
    let textures = GpuState::create_textures(device, queue, texture_layout, mesh);
    let identity = glam::Mat4::IDENTITY.to_cols_array_2d();
    let uniforms = Uniforms {
        mvp: (camera.projection_matrix() * camera.view_matrix()).to_cols_array_2d(),
        model: identity,
        normal_matrix: identity,
        light_dir: glam::Vec3::new(0.5, 1., 0.8).normalize().to_array(),
        _pad1: 0.,
        light_color: [1., 0.98, 0.95],
        _pad2: 0.,
        ambient: [0.18, 0.18, 0.22],
        _pad3: 0.,
        eye_pos: camera.eye().to_array(),
        edges: 0.,
    };
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::bytes_of(&uniforms),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: uniform_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: config.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let depth = GpuState::create_depth_texture(device, config, 1);
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.025,
                        g: 0.035,
                        b: 0.05,
                        a: 1.,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_bind_group(0, &binding, &[]);
        for chunk in GpuState::ordered_chunks(&chunks, camera) {
            pass.set_pipeline(&pipelines[chunk.topology]);
            pass.set_bind_group(1, &textures[chunk.texture], &[]);
            pass.set_vertex_buffer(0, chunk.vertices.slice(..));
            if let Some(indices) = &chunk.indices {
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..chunk.count, 0, 0..1);
            } else {
                pass.draw(0..chunk.count, 0..1);
            }
        }
    }
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (config.width * config.height * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::ImageCopyBuffer {
            buffer: &readback,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(config.width * 4),
                rows_per_image: Some(config.height),
            },
        },
        wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap()
        });
    device.poll(wgpu::Maintain::Wait);
    receive.recv().unwrap().unwrap();
    let data = readback.slice(..).get_mapped_range();
    let visible = data
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p.as_slice() != &data[..4])
        .count();
    assert!(visible > 0, "render contains only background");
    let mut ppm = format!("P6\n{} {}\n255\n", config.width, config.height).into_bytes();
    for pixel in data.as_chunks::<4>().0 {
        ppm.extend(&pixel[..3]);
    }
    std::fs::write(output, ppm).unwrap();
}
