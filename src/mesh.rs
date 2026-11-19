use glam::Vec3;

/// Vertex attributes laid out for GPU upload.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
    pub uv: [f32; 2],
    /// UVs for metallic/roughness, normal, occlusion, and emissive maps.
    pub map_uv: [[f32; 2]; 4],
    /// World-space tangent and bitangent handedness; zero means derive from UVs.
    pub tangent: [f32; 4],
}

impl Default for Vertex {
    fn default() -> Self {
        Self {
            position: [0.; 3],
            normal: [0.; 3],
            color: [0.7, 0.75, 0.8, 1.],
            uv: [0.; 2],
            map_uv: [[0.; 2]; 4],
            tangent: [0.; 4],
        }
    }
}

/// Alpha behavior is converted to the shader representation only at upload.
#[derive(Debug, Clone, Copy)]
pub enum AlphaMode {
    Opaque,
    Blend,
    Mask(f32),
}
impl AlphaMode {
    #[cfg(test)]
    pub fn cutoff(self) -> f32 {
        match self {
            Self::Mask(value) => value,
            _ => panic!("Expected masked material"),
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Material {
    pub metallic: f32,
    pub roughness: f32,
    pub alpha: AlphaMode,
    pub unlit: bool,
    pub emissive: [f32; 3],
    pub map_factors: [f32; 2],
}
impl Default for Material {
    fn default() -> Self {
        Self {
            metallic: 0.,
            roughness: 0.5,
            alpha: AlphaMode::Blend,
            unlit: false,
            emissive: [0.; 3],
            map_factors: [0., 1.],
        }
    }
}

#[derive(Debug, Clone)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub wrap: [u32; 2],
    pub linear: [bool; 2],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Topology {
    Triangles,
    Lines,
    Points,
}

#[derive(Debug, Clone)]
pub struct Batch {
    /// Original accessor indices in expanded draw order, when provided by the loader.
    pub source_indices: Option<std::sync::Arc<Vec<u32>>>,
    pub material: Material,
    pub range: std::ops::Range<usize>,
    pub topology: Topology,
    pub texture: Option<usize>,
    pub maps: [Option<usize>; 4],
    pub double_sided: bool,
    pub blend: bool,
}

/// Shared render geometry.
#[derive(Debug, Clone)]
pub struct Mesh {
    pub vertices: std::sync::Arc<Vec<Vertex>>,
    pub batches: Vec<Batch>,
    pub textures: std::sync::Arc<Vec<Texture>>,
    pub animation: Option<std::sync::Arc<crate::model::gltf::Scene>>,
    pub bounds_min: Vec3,
    pub bounds_max: Vec3,
}

impl Mesh {
    /// Empty mesh for when no model is loaded yet.
    pub fn empty() -> Self {
        Self {
            vertices: std::sync::Arc::new(Vec::new()),
            batches: Vec::new(),
            textures: Default::default(),
            animation: None,
            bounds_min: Vec3::ZERO,
            bounds_max: Vec3::ZERO,
        }
    }

    pub fn primitive_counts(&self) -> [usize; 3] {
        if self.batches.is_empty() {
            return [self.vertices.len() / 3, 0, 0];
        }
        let mut counts = [0; 3];
        for batch in &self.batches {
            counts[batch.topology as usize] +=
                batch.range.len() / [3, 2, 1][batch.topology as usize];
        }
        counts
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
pub const MAX_TRIANGLES: usize = 10_000_000;
pub const MAX_INPUT_BYTES: u64 = 512 * 1024 * 1024;
pub type MeshResult<T> = Result<T, Box<dyn std::error::Error>>;

// Scope restores the previous context, including when parsing fails or unwinds.
thread_local! { static LOAD_CONTEXT: std::cell::RefCell<crate::loading::LoadContext> = std::cell::RefCell::new(Default::default()); }
pub(crate) struct LoadingScope(crate::loading::LoadContext);
pub(crate) fn loading_scope(context: crate::loading::LoadContext) -> LoadingScope {
    LoadingScope(LOAD_CONTEXT.with(|current| current.replace(context)))
}
impl Drop for LoadingScope {
    fn drop(&mut self) {
        LOAD_CONTEXT.with(|current| current.replace(self.0.clone()));
    }
}
pub(crate) fn check_cancelled() -> MeshResult<()> {
    LOAD_CONTEXT.with(|context| context.borrow().check())
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
    #[cfg(test)]
    pub fn from_triangles(triangles: impl IntoIterator<Item = [[f32; 3]; 3]>) -> MeshResult<Self> {
        let mut mesh = Self::empty();
        mesh.bounds_min = Vec3::splat(f32::INFINITY);
        mesh.bounds_max = Vec3::splat(f32::NEG_INFINITY);
        for triangle in triangles {
            mesh.push_triangle(triangle)?;
        }
        mesh.finish()
    }

    pub(crate) fn triangle_builder(capacity: usize) -> Self {
        Self {
            vertices: std::sync::Arc::new(Vec::with_capacity(capacity.min(MAX_TRIANGLES) * 3)),
            batches: Vec::new(),
            textures: Default::default(),
            animation: None,
            bounds_min: Vec3::splat(f32::INFINITY),
            bounds_max: Vec3::splat(f32::NEG_INFINITY),
        }
    }

    pub(crate) fn push_triangle(&mut self, triangle: [[f32; 3]; 3]) -> MeshResult<()> {
        if self.vertices.len().is_multiple_of(3072) {
            check_cancelled()?;
        }
        if self.vertices.len() / 3 >= MAX_TRIANGLES {
            return Err("Model exceeds triangle limit".into());
        }
        if !triangle.iter().flatten().all(|v| v.is_finite()) {
            return Err("Non-finite vertex coordinate".into());
        }
        let [a, b, c] = triangle.map(|p| glam::DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64));
        let normal = (b - a)
            .cross(c - a)
            .normalize_or_zero()
            .as_vec3()
            .to_array();
        for position in triangle {
            let p = Vec3::from(position);
            self.bounds_min = self.bounds_min.min(p);
            self.bounds_max = self.bounds_max.max(p);
            std::sync::Arc::make_mut(&mut self.vertices).push(Vertex {
                position,
                normal,
                ..Default::default()
            });
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> MeshResult<Self> {
        if self.vertices.is_empty() {
            return Err("Model contains no renderable primitives".into());
        }
        if !self.extent().is_finite() || !self.center().is_finite() {
            return Err("Model coordinates exceed supported range".into());
        }
        Ok(self)
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

#[cfg(test)]
#[test]
fn clones_share_storage_and_mutations_preserve_original() {
    let original =
        Mesh::from_triangles([[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]]).unwrap();
    let mut copy = original.clone();
    assert!(std::sync::Arc::ptr_eq(&original.vertices, &copy.vertices));
    copy.push_triangle([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]])
        .unwrap();
    assert_eq!(original.vertices.len(), 3);
    assert_eq!(copy.vertices.len(), 6);
}
