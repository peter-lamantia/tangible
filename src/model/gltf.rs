use super::Result;
use crate::mesh::Mesh;
use glam::{Mat4, Vec3};
use std::path::Path;

pub(super) fn load(path: &Path) -> Result<Mesh> {
    // The viewer uses geometry only: importing images can expand compressed textures
    // into large pixel buffers that are immediately discarded.
    let source = gltf::Gltf::from_slice(&crate::mesh::read_limited(path)?)?;
    let doc = source.document;
    let mut blob = source.blob;
    let mut buffers = Vec::new();
    for buffer in doc.buffers() {
        let data = gltf::buffer::Data::from_source_and_blob(
            buffer.source(),
            path.parent(),
            &mut blob,
        )
        .map_err(|error| match buffer.source() {
            gltf::buffer::Source::Uri(uri) if !uri.starts_with("data:") => {
                let hint = if matches!(&error, gltf::Error::Io(e) if e.kind() == std::io::ErrorKind::NotFound) {
                    " Keep the .gltf file and its companion files together, preserving their relative paths, or open the .glb version instead."
                } else {
                    ""
                };
                format!("Could not load glTF buffer {uri:?} relative to {}: {error}.{hint}", path.parent().unwrap_or(Path::new(".")).display())
            }
            _ => format!("Could not load glTF buffer {}: {error}", buffer.index()),
        })?;
        // Preserve the validation performed by gltf::import_buffers.
        if data.len() < buffer.length() {
            return Err(gltf::Error::BufferLength {
                buffer: buffer.index(),
                expected: buffer.length(),
                actual: data.len(),
            }
            .into());
        }
        buffers.push(data);
    }
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
                        if !indices.len().is_multiple_of(3) {
                            return Err("Invalid glTF triangle count".into());
                        }
                        for c in indices.as_chunks::<3>().0 {
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
    fn missing_companion_buffer_names_resource_and_recovery() {
        let dir =
            std::env::temp_dir().join(format!("tangible-gltf-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Box.gltf");
        std::fs::write(
            &path,
            include_bytes!("../../tests/fixtures/gltf-sample-assets/Box/glTF/Box.gltf"),
        )
        .unwrap();
        let error = crate::model::load_model(&path).unwrap_err().to_string();
        assert!(error.contains("Box0.bin"), "{error}");
        assert!(error.contains(&dir.display().to_string()), "{error}");
        assert!(error.contains("companion files together"), "{error}");
        assert!(error.contains(".glb"), "{error}");
        // A truncated companion must still fail the buffer length validation.
        std::fs::write(dir.join("Box0.bin"), [0u8; 4]).unwrap();
        let error = load(&path).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<gltf::Error>(),
            Some(gltf::Error::BufferLength { .. })
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn imports_khronos_sample_assets() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gltf-sample-assets");
        let cases = [
            ("Triangle/glTF/Triangle.gltf", 1, [0.; 3], [1., 1., 0.]),
            (
                "Triangle/glTF-Embedded/Triangle.gltf",
                1,
                [0.; 3],
                [1., 1., 0.],
            ),
            (
                "TriangleWithoutIndices/glTF/TriangleWithoutIndices.gltf",
                1,
                [0.; 3],
                [1., 1., 0.],
            ),
            ("Box/glTF/Box.gltf", 12, [-0.5; 3], [0.5; 3]),
            ("Box/glTF-Binary/Box.glb", 12, [-0.5; 3], [0.5; 3]),
            (
                "BoxInterleaved/glTF/BoxInterleaved.gltf",
                12,
                [-0.5; 3],
                [0.5; 3],
            ),
            (
                "SimpleSparseAccessor/glTF/SimpleSparseAccessor.gltf",
                12,
                [0.; 3],
                [6., 4., 0.],
            ),
        ];
        for (file, triangles, min, max) in cases {
            // Exercise the same extension dispatch used when opening a model.
            let mesh = crate::model::load_model(root.join(file))
                .unwrap_or_else(|error| panic!("{file}: {error}"));
            assert_eq!(mesh.vertices.len(), triangles * 3, "{file}");
            assert!(
                mesh.bounds_min.abs_diff_eq(Vec3::from_array(min), 1e-6),
                "{file}: minimum bounds"
            );
            assert!(
                mesh.bounds_max.abs_diff_eq(Vec3::from_array(max), 1e-6),
                "{file}: maximum bounds"
            );
            for vertex in mesh.vertices.iter() {
                assert!(Vec3::from_array(vertex.position).is_finite(), "{file}");
                assert!(
                    (Vec3::from_array(vertex.normal).length() - 1.).abs() < 1e-6,
                    "{file}: unit normal"
                );
            }
        }
    }

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
