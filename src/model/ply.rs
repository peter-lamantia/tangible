use super::{obj::triangulate_face, Result};
use crate::mesh::{Mesh, MAX_TRIANGLES};
use ply_rs::{
    parser::Parser,
    ply::{DefaultElement, Property as P},
};

pub(super) fn load(path: &std::path::Path) -> Result<Mesh> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > crate::mesh::MAX_INPUT_BYTES {
        return Err("Model file exceeds 512 MiB limit".into());
    }
    parse_reader(std::io::BufReader::new(
        file.take(crate::mesh::MAX_INPUT_BYTES + 1),
    ))
}

#[cfg(test)]
fn parse(data: &[u8]) -> Result<Mesh> {
    parse_reader(std::io::Cursor::new(data))
}

fn parse_reader(mut input: impl std::io::BufRead) -> Result<Mesh> {
    let parser = Parser::<DefaultElement>::new();
    let header = parser.read_header(&mut input)?;
    let vertex_count = header
        .elements
        .get("vertex")
        .ok_or("PLY has no vertices")?
        .count;
    let face_count = header.elements.get("face").ok_or("PLY has no faces")?.count;
    if vertex_count > MAX_TRIANGLES * 3 || face_count > MAX_TRIANGLES {
        return Err("PLY exceeds geometry limit".into());
    }
    let scalar = |p: Option<&P>| -> Result<f32> {
        Ok(match p.ok_or("Missing PLY coordinate")? {
            P::Char(v) => *v as f32,
            P::UChar(v) => *v as f32,
            P::Short(v) => *v as f32,
            P::UShort(v) => *v as f32,
            P::Int(v) => *v as f32,
            P::UInt(v) => *v as f32,
            P::Float(v) => *v,
            P::Double(v) => *v as f32,
            _ => return Err("Invalid PLY coordinate".into()),
        })
    };
    let mut points = Vec::with_capacity(vertex_count);
    let mut mesh = Mesh::triangle_builder(face_count);
    let mut line = String::new();
    for (name, element) in &header.elements {
        for i in 0..element.count {
            if i % 1024 == 0 {
                crate::mesh::check_cancelled()?;
            }
            // Retain only one property record at a time, rather than millions of maps.
            let record = match header.encoding {
                ply_rs::ply::Encoding::Ascii => {
                    line.clear();
                    if input.read_line(&mut line)? == 0 {
                        return Err("Truncated PLY payload".into());
                    }
                    parser.read_ascii_element(line.trim(), element)?
                }
                ply_rs::ply::Encoding::BinaryLittleEndian => {
                    parser.read_little_endian_element(&mut input, element)?
                }
                ply_rs::ply::Encoding::BinaryBigEndian => {
                    parser.read_big_endian_element(&mut input, element)?
                }
            };
            if name == "vertex" {
                let point = [
                    scalar(record.get("x"))?,
                    scalar(record.get("y"))?,
                    scalar(record.get("z"))?,
                ];
                if !point.iter().all(|v| v.is_finite()) {
                    return Err("Non-finite PLY coordinate".into());
                }
                points.push(point);
                continue;
            }
            if name != "face" {
                continue;
            }
            if points.len() != vertex_count {
                return Err("PLY faces must follow vertices".into());
            }
            let face = &record;
            let prop = face
                .get("vertex_indices")
                .or_else(|| face.get("vertex_index"))
                .ok_or("Missing PLY indices")?;
            let ids: Vec<i64> = match prop {
                P::ListChar(v) => v.iter().map(|x| *x as i64).collect(),
                P::ListUChar(v) => v.iter().map(|x| *x as i64).collect(),
                P::ListShort(v) => v.iter().map(|x| *x as i64).collect(),
                P::ListUShort(v) => v.iter().map(|x| *x as i64).collect(),
                P::ListInt(v) => v.iter().map(|x| *x as i64).collect(),
                P::ListUInt(v) => v.iter().map(|x| *x as i64).collect(),
                _ => return Err("Invalid PLY indices".into()),
            };
            if !(3..=4096).contains(&ids.len())
                || mesh.vertices.len() / 3 + ids.len() - 2 > MAX_TRIANGLES
            {
                return Err("Invalid or excessive PLY face".into());
            }
            let face: Vec<_> = ids
                .iter()
                .map(|&i| {
                    usize::try_from(i)
                        .ok()
                        .and_then(|i| points.get(i))
                        .copied()
                        .ok_or("PLY index out of range")
                })
                .collect::<std::result::Result<_, _>>()?;
            if face.len() == 3 {
                mesh.push_triangle([face[0], face[1], face[2]])?;
            } else {
                for triangle in triangulate_face(&face)? {
                    mesh.push_triangle(triangle)?;
                }
            }
        }
    }
    mesh.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "set TANGIBLE_PLY_BENCH to a PLY path to measure loading"]
    fn measure_file_load() {
        let path = std::env::var("TANGIBLE_PLY_BENCH").expect("set TANGIBLE_PLY_BENCH");
        let mesh = load(std::path::Path::new(&path)).unwrap();
        println!(
            "{} triangles; {} bytes of mesh storage",
            mesh.vertices.len() / 3,
            mesh.vertices.capacity() * std::mem::size_of::<crate::mesh::Vertex>()
        );
    }
    #[test]
    fn ascii_triangle() {
        let data = b"ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";
        assert_eq!(parse(data).unwrap().vertices.len(), 3);
    }
    #[test]
    fn binary_triangle() {
        let mut data = b"ply\nformat binary_little_endian 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n".to_vec();
        for f in [0f32, 0., 0., 1., 0., 0., 0., 1., 0.] {
            data.extend(f.to_le_bytes());
        }
        data.push(3);
        for i in [0i32, 1, 2] {
            data.extend(i.to_le_bytes());
        }
        assert_eq!(parse(&data).unwrap().vertices.len(), 3);
    }
}
