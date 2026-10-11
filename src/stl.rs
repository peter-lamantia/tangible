use glam::Vec3;

/// A single vertex with position and normal, laid out for GPU upload.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

/// Parsed STL mesh data.
#[derive(Debug, Clone)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub bounds_min: Vec3,
    pub bounds_max: Vec3,
}

impl Mesh {
    /// Empty mesh for when no model is loaded yet.
    pub fn empty() -> Self {
        Self {
            vertices: Vec::new(),
            bounds_min: Vec3::ZERO,
            bounds_max: Vec3::ZERO,
        }
    }

    /// Center point of the bounding box.
    pub fn center(&self) -> Vec3 {
        (self.bounds_min + self.bounds_max) * 0.5
    }

    /// Diagonal length of the bounding box.
    pub fn extent(&self) -> f32 {
        (self.bounds_max - self.bounds_min).length()
    }

    /// Dimensions (width, height, depth) of the bounding box.
    pub fn dimensions(&self) -> Vec3 {
        (self.bounds_max - self.bounds_min).abs()
    }
}

/// Load an STL file (auto-detects binary vs ASCII).
pub fn load_stl(path: impl AsRef<std::path::Path>) -> Result<Mesh, Box<dyn std::error::Error>> {
    let data = std::fs::read(path)?;

    // Heuristic: try binary first (most STL files are binary)
    if data.len() > 84 {
        if let Ok(mesh) = parse_binary_stl(&data) {
            return Ok(mesh);
        }
    }

    parse_ascii_stl(&data)
}

fn read_f32(data: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn parse_binary_stl(data: &[u8]) -> Result<Mesh, Box<dyn std::error::Error>> {
    if data.len() < 84 {
        return Err("File too small for binary STL".into());
    }

    let num_triangles =
        u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as usize;
    let expected_size = 84 + num_triangles * 50;

    if data.len() != expected_size {
        return Err(format!(
            "Binary STL size mismatch: expected {expected_size}, got {}",
            data.len()
        )
        .into());
    }

    let mut vertices = Vec::with_capacity(num_triangles * 3);
    let mut bounds_min = Vec3::splat(f32::MAX);
    let mut bounds_max = Vec3::splat(f32::MIN);

    for i in 0..num_triangles {
        let base = 84 + i * 50;

        // Read face normal
        let mut normal = [
            read_f32(data, base),
            read_f32(data, base + 4),
            read_f32(data, base + 8),
        ];

        // Read 3 vertices
        let mut face_verts = [[0.0f32; 3]; 3];
        for v in 0..3 {
            let vo = base + 12 + v * 12;
            face_verts[v] = [
                read_f32(data, vo),
                read_f32(data, vo + 4),
                read_f32(data, vo + 8),
            ];
        }

        // Recompute normal if the stored one is degenerate
        let n = Vec3::from(normal);
        if n.length_squared() < 1e-10 {
            let v0 = Vec3::from(face_verts[0]);
            let v1 = Vec3::from(face_verts[1]);
            let v2 = Vec3::from(face_verts[2]);
            let computed = (v1 - v0).cross(v2 - v0).normalize_or_zero();
            normal = computed.to_array();
        }

        for pos in &face_verts {
            let p = Vec3::from(*pos);
            bounds_min = bounds_min.min(p);
            bounds_max = bounds_max.max(p);
            vertices.push(Vertex {
                position: *pos,
                normal,
            });
        }
    }

    Ok(Mesh {
        vertices,
        bounds_min,
        bounds_max,
    })
}

fn parse_ascii_stl(data: &[u8]) -> Result<Mesh, Box<dyn std::error::Error>> {
    let text = std::str::from_utf8(data)?;
    let mut vertices = Vec::new();
    let mut bounds_min = Vec3::splat(f32::MAX);
    let mut bounds_max = Vec3::splat(f32::MIN);

    let mut current_normal = [0.0f32; 3];
    let mut face_verts: Vec<[f32; 3]> = Vec::new();

    for line in text.lines() {
        let line = line.trim();

        if let Some(rest) = line.strip_prefix("facet normal") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 3 {
                current_normal = [
                    parts[0].parse()?,
                    parts[1].parse()?,
                    parts[2].parse()?,
                ];
            }
            face_verts.clear();
        } else if let Some(rest) = line.strip_prefix("vertex") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 3 {
                let pos: [f32; 3] = [
                    parts[0].parse()?,
                    parts[1].parse()?,
                    parts[2].parse()?,
                ];

                let p = Vec3::from(pos);
                bounds_min = bounds_min.min(p);
                bounds_max = bounds_max.max(p);

                face_verts.push(pos);
            }
        } else if line.starts_with("endfacet") {
            // Recompute normal if degenerate
            let n = Vec3::from(current_normal);
            let mut normal = current_normal;
            if n.length_squared() < 1e-10 && face_verts.len() >= 3 {
                let v0 = Vec3::from(face_verts[0]);
                let v1 = Vec3::from(face_verts[1]);
                let v2 = Vec3::from(face_verts[2]);
                let computed = (v1 - v0).cross(v2 - v0).normalize_or_zero();
                normal = computed.to_array();
            }

            for pos in &face_verts {
                vertices.push(Vertex {
                    position: *pos,
                    normal,
                });
            }
        }
    }

    if vertices.is_empty() {
        return Err("No vertices found in ASCII STL".into());
    }

    Ok(Mesh {
        vertices,
        bounds_min,
        bounds_max,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_ascii_cube() {
        let mesh = load_stl("test_cube.stl").expect("Failed to load test_cube.stl");
        assert_eq!(mesh.vertices.len(), 36); // 12 triangles * 3 vertices
        assert_eq!(mesh.bounds_min, Vec3::new(0.0, 0.0, 0.0));
        assert_eq!(mesh.bounds_max, Vec3::new(1.0, 1.0, 1.0));
        assert_eq!(mesh.center(), Vec3::new(0.5, 0.5, 0.5));
    }
}
