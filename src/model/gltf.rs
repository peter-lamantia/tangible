use super::Result;
use crate::mesh::Mesh;
use glam::{Mat4, Vec3};
use std::path::Path;

pub(super) fn load(path: &Path) -> Result<Mesh> {
    // The viewer uses geometry only: importing images can expand compressed textures
    // into large pixel buffers that are immediately discarded.
    let source = gltf::Gltf::from_slice(&crate::mesh::read_limited(path)?)?;
    let doc = source.document;
    let buffers = gltf::import_buffers(&doc, path.parent(), source.blob)?;
    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .ok_or("glTF has no scene")?;
    let mut output = Mesh::triangle_builder(0);
    let mut stack: Vec<_> = scene.nodes().map(|n| (n, Mat4::IDENTITY, 0usize)).collect();
    while let Some((node, parent, depth)) = stack.pop() {
        crate::mesh::check_cancelled()?;
        if depth > 256 {
            return Err("glTF node hierarchy too deep".into());
        }
        let transform = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        stack.extend(node.children().map(|n| (n, transform, depth + 1)));
        if let Some(mesh) = node.mesh() {
            for primitive in mesh.primitives() {
                let reader = primitive.reader(|b| buffers.get(b.index()).map(|b| b.0.as_slice()));
                let points: Vec<_> = reader
                    .read_positions()
                    .ok_or("glTF primitive has no positions")?
                    .map(|p| transform.transform_point3(Vec3::from_array(p)).to_array())
                    .collect();
                let indices: Vec<usize> = match reader.read_indices() {
                    Some(v) => v.into_u32().map(|v| v as usize).collect(),
                    None => (0..points.len()).collect(),
                };
                let mut push = |a: usize, b: usize, c: usize| -> Result<()> {
                    output.push_triangle([
                        *points.get(a).ok_or("glTF index out of range")?,
                        *points.get(b).ok_or("glTF index out of range")?,
                        *points.get(c).ok_or("glTF index out of range")?,
                    ])?;
                    Ok(())
                };
                match primitive.mode() {
                    gltf::mesh::Mode::Triangles => {
                        if indices.len() % 3 != 0 {
                            return Err("Invalid glTF triangle count".into());
                        }
                        for c in indices.chunks_exact(3) {
                            push(c[0], c[1], c[2])?;
                        }
                    }
                    gltf::mesh::Mode::TriangleStrip => {
                        for (i, c) in indices.windows(3).enumerate() {
                            if i % 2 == 0 {
                                push(c[0], c[1], c[2])?
                            } else {
                                push(c[1], c[0], c[2])?
                            }
                        }
                    }
                    gltf::mesh::Mode::TriangleFan => {
                        for c in indices.get(1..).unwrap_or(&[]).windows(2) {
                            push(indices[0], c[0], c[1])?;
                        }
                    }
                    _ => return Err("glTF point and line primitives are not supported".into()),
                }
            }
        }
    }
    output.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imports_transformed_external_buffer() {
        let dir = std::env::temp_dir().join(format!("tangible-gltf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut bytes = Vec::new();
        for value in [0f32, 0., 0., 1., 0., 0., 0., 1., 0.] {
            bytes.extend(value.to_le_bytes());
        }
        std::fs::write(dir.join("mesh.bin"), bytes).unwrap();
        std::fs::write(dir.join("mesh.gltf"), r#"{"asset":{"version":"2.0"},"buffers":[{"uri":"mesh.bin","byteLength":36}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"images":[{"uri":"unused-texture.png"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],"nodes":[{"mesh":0,"translation":[2,0,0]}],"scenes":[{"nodes":[0]}],"scene":0}"#).unwrap();
        let mesh = load(&dir.join("mesh.gltf")).unwrap();
        assert_eq!(mesh.bounds_min.x, 2.0);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
