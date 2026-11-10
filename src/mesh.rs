use glam::Vec3;

/// A single vertex with position and normal, laid out for GPU upload.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

/// Shared triangle mesh data.
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

// Limits apply before allocating expanded geometry or reading whole input files.
pub const MAX_TRIANGLES: usize = 5_000_000;
pub const MAX_INPUT_BYTES: u64 = 512 * 1024 * 1024;
pub type MeshResult<T> = Result<T, Box<dyn std::error::Error>>;

thread_local! { static REQUEST_ID: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) }; }
pub(crate) fn set_request_id(id: u64) {
    REQUEST_ID.with(|r| r.set(Some(id)));
}
pub(crate) fn check_cancelled() -> MeshResult<()> {
    if REQUEST_ID.with(|r| r.get().is_some_and(|id| !crate::loading::is_current(id))) {
        Err("Loading cancelled".into())
    } else {
        Ok(())
    }
}

pub fn read_limited(path: &std::path::Path) -> MeshResult<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > MAX_INPUT_BYTES {
        return Err("Model file exceeds 512 MiB limit".into());
    }
    let mut data = Vec::new();
    let mut input = file.take(MAX_INPUT_BYTES + 1);
    let mut chunk = [0; 65536];
    loop {
        check_cancelled()?;
        let n = input.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..n]);
    }
    if data.len() as u64 > MAX_INPUT_BYTES {
        return Err("Model file exceeds 512 MiB limit".into());
    }
    Ok(data)
}

impl Mesh {
    pub fn from_triangles(triangles: impl IntoIterator<Item = [[f32; 3]; 3]>) -> MeshResult<Self> {
        let mut mesh = Self::empty();
        mesh.bounds_min = Vec3::splat(f32::INFINITY);
        mesh.bounds_max = Vec3::splat(f32::NEG_INFINITY);
        for triangle in triangles {
            if mesh.vertices.len().is_multiple_of(3072) {
                check_cancelled()?;
            }
            if mesh.vertices.len() / 3 >= MAX_TRIANGLES {
                return Err("Model exceeds triangle limit".into());
            }
            if !triangle.iter().flatten().all(|v| v.is_finite()) {
                return Err("Non-finite vertex coordinate".into());
            }
            let [a, b, c] =
                triangle.map(|p| glam::DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64));
            let normal = (b - a)
                .cross(c - a)
                .normalize_or_zero()
                .as_vec3()
                .to_array();
            for position in triangle {
                let p = Vec3::from(position);
                mesh.bounds_min = mesh.bounds_min.min(p);
                mesh.bounds_max = mesh.bounds_max.max(p);
                mesh.vertices.push(Vertex { position, normal });
            }
        }
        if mesh.vertices.is_empty() {
            return Err("Model contains no triangles".into());
        }
        if !mesh.extent().is_finite() || !mesh.center().is_finite() {
            return Err("Model coordinates exceed supported range".into());
        }
        Ok(mesh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_empty_nonfinite_and_unrepresentable_bounds() {
        assert!(Mesh::from_triangles([]).is_err());
        for value in [f32::NAN, f32::INFINITY, f32::MAX] {
            assert!(Mesh::from_triangles([[
                [value, 0.0, 0.0],
                [-value, 1.0, 0.0],
                [0.0, 0.0, 1.0]
            ]])
            .is_err());
        }
    }
    #[test]
    fn large_finite_triangles_produce_finite_normals() {
        let mesh =
            Mesh::from_triangles([[[0.0, 0.0, 0.0], [1e15, 0.0, 0.0], [0.0, 1e15, 0.0]]]).unwrap();
        assert_eq!(mesh.vertices[0].normal, [0.0, 0.0, 1.0]);
    }
}
