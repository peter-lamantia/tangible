use crate::mesh::{read_limited, Mesh, MeshResult, MAX_TRIANGLES};

/// Load binary or ASCII STL, validating complete triangles in either encoding.
pub fn load_stl(path: impl AsRef<std::path::Path>) -> MeshResult<Mesh> {
    let data = read_limited(path.as_ref())?;
    if data.len() >= 84 {
        let count = u32::from_le_bytes(data[80..84].try_into()?) as usize;
        if count.checked_mul(50).and_then(|n| n.checked_add(84)) == Some(data.len()) {
            return parse_binary_stl(&data);
        }
    }
    parse_ascii_stl(&data)
}

fn parse_binary_stl(data: &[u8]) -> MeshResult<Mesh> {
    if data.len() < 84 {
        return Err("File too small for binary STL".into());
    }
    let count = u32::from_le_bytes(data[80..84].try_into()?) as usize;
    if count > MAX_TRIANGLES {
        return Err("STL exceeds triangle limit".into());
    }
    if count.checked_mul(50).and_then(|n| n.checked_add(84)) != Some(data.len()) {
        return Err("Binary STL size mismatch".into());
    }
    let mut triangles = Vec::with_capacity(count);
    for record in data[84..].as_chunks::<50>().0 {
        if triangles.len() % 1024 == 0 {
            crate::mesh::check_cancelled()?;
        }
        let mut triangle = [[0.0; 3]; 3];
        for (i, p) in triangle.iter_mut().enumerate() {
            for (axis, value) in p.iter_mut().enumerate() {
                let offset = 12 + i * 12 + axis * 4;
                *value = f32::from_le_bytes(record[offset..offset + 4].try_into()?);
            }
        }
        triangles.push(triangle);
    }
    // Derive normals from geometry; file normals are frequently missing or invalid.
    Mesh::from_triangles(triangles)
}

fn parse_ascii_stl(data: &[u8]) -> MeshResult<Mesh> {
    let text = std::str::from_utf8(data)?;
    let mut triangles = Vec::new();
    let mut face: Option<Vec<[f32; 3]>> = None;
    let mut in_loop = false;
    let mut loop_done = false;
    let mut solid = false;
    let mut ended = false;
    for (line_no, line) in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .enumerate()
    {
        if line_no % 1024 == 0 {
            crate::mesh::check_cancelled()?;
        }
        let mut fields = line.split_whitespace();
        match fields.next().unwrap_or("") {
            "solid" if !solid && !ended => solid = true,
            "facet" if solid && face.is_none() => {
                if fields.next() != Some("normal") {
                    return Err("Invalid STL facet normal".into());
                }
                let normal = fields
                    .map(str::parse::<f32>)
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                if normal.len() != 3 {
                    return Err("Invalid STL facet normal".into());
                }
                face = Some(Vec::with_capacity(3));
                loop_done = false;
            }
            "outer" if face.is_some() && !in_loop && !loop_done => {
                if fields.next() != Some("loop") || fields.next().is_some() {
                    return Err("Invalid STL outer loop".into());
                }
                in_loop = true;
            }
            "vertex" if in_loop => {
                let values = fields
                    .map(str::parse::<f32>)
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                if values.len() != 3 || !values.iter().all(|v| v.is_finite()) {
                    return Err("Invalid STL vertex".into());
                }
                let vertices = face.as_mut().ok_or("STL vertex outside facet")?;
                if vertices.len() >= 3 {
                    return Err("STL facet must have exactly three vertices".into());
                }
                vertices.push([values[0], values[1], values[2]]);
            }
            "endloop" if in_loop => {
                in_loop = false;
                loop_done = true;
            }
            "endfacet" if face.is_some() && loop_done && !in_loop => {
                let vertices = face.take().ok_or("Missing STL facet")?;
                let triangle: [[f32; 3]; 3] = vertices
                    .try_into()
                    .map_err(|_| "STL facet must have exactly three vertices")?;
                if triangles.len() >= MAX_TRIANGLES {
                    return Err("STL exceeds triangle limit".into());
                }
                triangles.push(triangle);
            }
            "endsolid" if solid && face.is_none() => {
                solid = false;
                ended = true;
            }
            _ => return Err(format!("Unexpected or incomplete STL statement: {line}").into()),
        }
    }
    if face.is_some() || in_loop || solid || !ended {
        return Err("Incomplete ASCII STL".into());
    }
    Mesh::from_triangles(triangles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn binary_triangle() -> Vec<u8> {
        let mut data = vec![0; 134];
        data[80..84].copy_from_slice(&1u32.to_le_bytes());
        data[108..112].copy_from_slice(&1f32.to_le_bytes());
        data[124..128].copy_from_slice(&1f32.to_le_bytes());
        data
    }

    #[test]
    fn binary_validation_and_recomputed_normals() {
        let mut data = binary_triangle();
        data[84..88].copy_from_slice(&f32::NAN.to_le_bytes());
        let mesh = parse_binary_stl(&data).unwrap();
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.vertices[0].normal, [0.0, 0.0, 1.0]);
        data[96..100].copy_from_slice(&f32::INFINITY.to_le_bytes());
        assert!(parse_binary_stl(&data).is_err());
        assert!(parse_binary_stl(&data[..100]).is_err());
        let mut empty = vec![0; 84];
        assert!(parse_binary_stl(&empty).is_err());
        empty[80..84].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_binary_stl(&empty).is_err());
    }

    #[test]
    fn ascii_rejects_malformed_facets_and_nonfinite_vertices() {
        let valid = "solid test\nfacet normal 0 0 0\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid test";
        assert!(parse_ascii_stl(valid.as_bytes()).is_ok());
        for invalid in [
            valid.replace("vertex 0 1 0\n", ""),
            valid.replace("endloop", "vertex 0 0 1\nendloop"),
            valid.replace("vertex 1 0 0", "vertex NaN 0 0"),
            valid.replace("endfacet", ""),
            valid.replace("endsolid test", ""),
        ] {
            assert!(parse_ascii_stl(invalid.as_bytes()).is_err(), "{invalid}");
        }
    }

    #[test]
    fn test_load_ascii_cube() {
        let mesh = load_stl("test_cube.stl").expect("Failed to load test_cube.stl");
        assert_eq!(mesh.vertices.len(), 36); // 12 triangles * 3 vertices
        assert_eq!(mesh.bounds_min, Vec3::new(0.0, 0.0, 0.0));
        assert_eq!(mesh.bounds_max, Vec3::new(1.0, 1.0, 1.0));
        assert_eq!(mesh.center(), Vec3::new(0.5, 0.5, 0.5));
    }
}
