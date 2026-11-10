use crate::{camera, mesh};
pub(crate) fn plate_preview(
    mesh: &mesh::Mesh,
    current: impl Fn() -> bool,
) -> Option<egui::ColorImage> {
    const SIZE: usize = 400;
    let background = egui::Color32::TRANSPARENT;
    let mut image = egui::ColorImage::new([SIZE, SIZE], background);
    if mesh.vertices.is_empty() {
        return current().then_some(image);
    }
    const SAMPLES: [(f32, f32); 4] = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];
    let mut depth = vec![f32::INFINITY; SIZE * SIZE * SAMPLES.len()];
    let mut colors = vec![background; SIZE * SIZE * SAMPLES.len()];
    let mut camera = camera::Camera::new(mesh.center(), 5.0);
    camera.aspect = 1.0;
    camera.frame_target(mesh.center(), mesh.extent());
    let mvp = camera.projection_matrix() * camera.view_matrix();
    let light = glam::Vec3::new(0.5, 1.0, 0.8).normalize();
    let eye = camera.eye();
    for triangle in mesh.vertices.chunks_exact(3) {
        if !current() {
            return None;
        }
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
            if !current() {
                return None;
            }
            for x in x_min..=x_max {
                for (sample, (dx, dy)) in SAMPLES.iter().enumerate() {
                    let px = x as f32 + dx;
                    let py = y as f32 + dy;
                    let w0 = edge(screen[1], screen[2], px, py) / area;
                    let w1 = edge(screen[2], screen[0], px, py) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let z = w0 * screen[0][2] + w1 * screen[1][2] + w2 * screen[2][2];
                    let index = (y * SIZE + x) * SAMPLES.len() + sample;
                    if z < depth[index] {
                        depth[index] = z;
                        colors[index] = color;
                    }
                }
            }
        }
    }
    for (pixel, samples) in image
        .pixels
        .iter_mut()
        .zip(colors.chunks_exact(SAMPLES.len()))
    {
        let mut sum = [0u32; 4];
        for sample in samples {
            for (total, channel) in sum.iter_mut().zip(sample.to_array()) {
                *total += u32::from(channel);
            }
        }
        *pixel = egui::Color32::from_rgba_premultiplied(
            (sum[0] / 4) as u8,
            (sum[1] / 4) as u8,
            (sum[2] / 4) as u8,
            (sum[3] / 4) as u8,
        );
    }
    current().then_some(image)
}

#[cfg(test)]
#[test]
fn plate_preview_renders_geometry() {
    let mesh = mesh::Mesh {
        vertices: vec![
            mesh::Vertex {
                position: [-1.0, -1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
            },
            mesh::Vertex {
                position: [1.0, -1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
            },
            mesh::Vertex {
                position: [0.0, 1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
            },
        ],
        bounds_min: glam::Vec3::new(-1.0, -1.0, 0.0),
        bounds_max: glam::Vec3::new(1.0, 1.0, 0.0),
    };
    let preview = plate_preview(&mesh, || true).unwrap();
    assert_eq!(preview.size, [400, 400]);
    assert!(preview
        .pixels
        .iter()
        .any(|pixel| *pixel != egui::Color32::TRANSPARENT));
}

#[cfg(test)]
#[test]
fn cancelled_preview_stops_before_rasterization() {
    let mesh =
        mesh::Mesh::from_triangles([[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]]).unwrap();
    assert!(plate_preview(&mesh, || false).is_none());
}
