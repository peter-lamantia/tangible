use super::Result;
use crate::mesh::{Mesh, MAX_TRIANGLES};
pub(super) fn parse_obj(text: &str) -> Result<Mesh> {
    let mut points = Vec::<[f32; 3]>::new();
    let mut mesh = Mesh::triangle_builder(0);
    for (line_no, line) in text.lines().enumerate() {
        if line_no % 1024 == 0 {
            crate::mesh::check_cancelled()?;
        }
        let line = line.split('#').next().unwrap_or("").trim();
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("v") => {
                let p: Vec<f32> = parts
                    .take(3)
                    .map(str::parse)
                    .collect::<std::result::Result<_, _>>()?;
                if p.len() != 3 {
                    return Err(format!("OBJ line {}: incomplete vertex", line_no + 1).into());
                }
                if !p.iter().all(|v| v.is_finite()) {
                    return Err("Non-finite OBJ vertex".into());
                }
                if points.len() >= MAX_TRIANGLES * 3 {
                    return Err("OBJ exceeds vertex limit".into());
                }
                points.push([p[0], p[1], p[2]]);
            }
            Some("f") => {
                let mut face = Vec::new();
                for part in parts {
                    let index: isize = part.split('/').next().unwrap_or("").parse()?;
                    if index == 0 {
                        return Err(
                            format!("OBJ line {}: index zero is invalid", line_no + 1).into()
                        );
                    }
                    let index = if index > 0 {
                        index - 1
                    } else {
                        points.len() as isize + index
                    };
                    if index < 0 || index as usize >= points.len() {
                        return Err(
                            format!("OBJ line {}: vertex index out of range", line_no + 1).into(),
                        );
                    }
                    if face.len() >= 4096 {
                        return Err("OBJ face exceeds 4096 vertex limit".into());
                    }
                    face.push(points[index as usize]);
                }
                if face.len() < 3 {
                    return Err(format!(
                        "OBJ line {}: face has fewer than three vertices",
                        line_no + 1
                    )
                    .into());
                }
                if mesh.vertices.len() / 3 + face.len() - 2 > MAX_TRIANGLES {
                    return Err("OBJ exceeds triangle limit".into());
                }
                for triangle in
                    triangulate_face(&face).map_err(|e| format!("OBJ line {}: {e}", line_no + 1))?
                {
                    mesh.push_triangle(triangle)?;
                }
            }
            _ => {}
        }
    }
    mesh.finish()
}

// Ear clipping preserves the interior of simple concave polygons in either winding.
pub(super) fn triangulate_face(face: &[[f32; 3]]) -> Result<Vec<[[f32; 3]; 3]>> {
    let points: Vec<_> = face
        .iter()
        .map(|p| glam::DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64))
        .collect();
    let origin = points[0];
    let mut normal = glam::DVec3::ZERO;
    for i in 1..points.len() - 1 {
        normal += (points[i] - origin).cross(points[i + 1] - origin);
    }
    let axis = normal.abs().max_element();
    if axis == 0.0 {
        return Err("Degenerate OBJ polygon".into());
    }
    let projected: Vec<_> = points
        .iter()
        .map(|p| {
            let p = *p - origin;
            if normal.x.abs() == axis {
                [p.y, p.z]
            } else if normal.y.abs() == axis {
                [p.x, p.z]
            } else {
                [p.x, p.y]
            }
        })
        .collect();
    let cross = |a: usize, b: usize, c: usize| {
        let [ax, ay] = projected[a];
        let [bx, by] = projected[b];
        let [cx, cy] = projected[c];
        (bx - ax) * (cy - ay) - (by - ay) * (cx - ax)
    };
    let area: f64 = (1..face.len() - 1).map(|i| cross(0, i, i + 1)).sum();
    let winding = area.signum();
    let epsilon = area.abs() * 1e-12;
    let mut remaining: Vec<usize> = (0..face.len()).collect();
    let mut result = Vec::new();
    while remaining.len() > 3 {
        crate::mesh::check_cancelled()?;
        let mut clipped = false;
        for i in 0..remaining.len() {
            let a = remaining[(i + remaining.len() - 1) % remaining.len()];
            let b = remaining[i];
            let c = remaining[(i + 1) % remaining.len()];
            let turn = cross(a, b, c) * winding;
            if turn.abs() <= epsilon {
                remaining.remove(i);
                clipped = true;
                break;
            }
            if turn < 0.0 {
                continue;
            }
            if remaining.iter().copied().any(|p| {
                p != a
                    && p != b
                    && p != c
                    && cross(a, b, p) * winding >= -epsilon
                    && cross(b, c, p) * winding >= -epsilon
                    && cross(c, a, p) * winding >= -epsilon
            }) {
                continue;
            }
            result.push([face[a], face[b], face[c]]);
            remaining.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            return Err("OBJ polygon cannot be triangulated (possibly self-intersecting)".into());
        }
    }
    if remaining.len() == 3 && cross(remaining[0], remaining[1], remaining[2]) * winding > epsilon {
        result.push([face[remaining[0]], face[remaining[1]], face[remaining[2]]]);
    }
    if result.is_empty() {
        return Err("Degenerate OBJ polygon".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    #[test]
    fn concave_obj_preserves_area_and_winding() {
        let points = "v 0 0 0\nv 3 0 0\nv 3 3 0\nv 2 3 0\nv 2 1 0\nv 1 1 0\nv 1 3 0\nv 0 3 0\n";
        for (face, sign) in [("f 1 2 3 4 5 6 7 8", 1.0), ("f 8 7 6 5 4 3 2 1", -1.0)] {
            let mesh = parse_obj(&format!("{points}{face}")).unwrap();
            assert_eq!(mesh.vertices.len(), 18);
            let mut area = 0.0;
            for triangle in mesh.vertices.as_chunks::<3>().0 {
                let [a, b, c] = [0, 1, 2].map(|i| Vec3::from(triangle[i].position));
                let signed = (b - a).cross(c - a).z * 0.5;
                assert!(signed * sign > 0.0);
                area += signed;
            }
            assert!((area - sign * 7.0).abs() < 1e-5);
        }
    }
    #[test]
    fn rejects_invalid_obj_coordinates_and_faces() {
        for text in [
            "v NaN 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3",
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 1 2",
            "v 0 0 0\nv 1 0 0\nv 2 0 0\nf 1 2 3",
        ] {
            assert!(parse_obj(text).is_err());
        }
    }
    #[test]
    fn obj_quad_and_negative_indices() {
        let mesh = parse_obj("v 0 0 0\nv 2 0 0\nv 2 2 0\nv 0 2 0\nf -4 -3 -2 -1\n").unwrap();
        assert_eq!(mesh.vertices.len(), 6);
        assert_eq!(mesh.bounds_max, Vec3::new(2.0, 2.0, 0.0));
        assert_eq!(mesh.vertices[0].normal, [0.0, 0.0, 1.0]);
    }
    #[test]
    fn obj_rejects_bad_index() {
        assert!(parse_obj("v 0 0 0\nf 1 2 3").is_err());
    }
}
