use super::{obj::triangulate_face, Result};
use crate::mesh::{Mesh, MAX_TRIANGLES};

pub(super) fn parse_off(text: &str) -> Result<Mesh> {
    let mut lines = text
        .lines()
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .filter(|line| !line.is_empty());
    let header = lines.next().ok_or("Missing OFF header")?;
    let counts = if header == "OFF" {
        lines.next().ok_or("Missing OFF counts")?
    } else {
        header.strip_prefix("OFF ").ok_or("Invalid OFF header")?
    };
    let counts: Vec<usize> = counts
        .split_whitespace()
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    if counts.len() != 3 || counts[0] > MAX_TRIANGLES * 3 || counts[1] > MAX_TRIANGLES {
        return Err("Invalid or excessive OFF counts".into());
    }
    let mut vertices = Vec::with_capacity(counts[0]);
    for _ in 0..counts[0] {
        let values: Vec<f32> = lines
            .next()
            .ok_or("Missing OFF vertex")?
            .split_whitespace()
            .take(3)
            .map(str::parse)
            .collect::<std::result::Result<_, _>>()?;
        if values.len() != 3 || !values.iter().all(|v| v.is_finite()) {
            return Err("Invalid OFF vertex".into());
        }
        vertices.push([values[0], values[1], values[2]]);
    }
    let mut mesh = Mesh::triangle_builder(counts[1]);
    for i in 0..counts[1] {
        if i % 1024 == 0 {
            crate::mesh::check_cancelled()?;
        }
        let mut fields = lines.next().ok_or("Missing OFF face")?.split_whitespace();
        let count: usize = fields.next().ok_or("Missing OFF face size")?.parse()?;
        if !(3..=4096).contains(&count) || mesh.vertices.len() / 3 + count - 2 > MAX_TRIANGLES {
            return Err("Invalid or excessive OFF face".into());
        }
        let mut face = Vec::new();
        for _ in 0..count {
            let index: usize = fields.next().ok_or("Missing OFF index")?.parse()?;
            face.push(*vertices.get(index).ok_or("OFF index out of range")?);
        }
        for triangle in triangulate_face(&face)? {
            mesh.push_triangle(triangle)?;
        }
    }
    mesh.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loads_quad() {
        assert_eq!(
            parse_off("OFF\n4 1 0\n0 0 0\n1 0 0\n1 1 0\n0 1 0\n4 0 1 2 3\n")
                .unwrap()
                .vertices
                .len(),
            6
        );
    }
    #[test]
    fn rejects_bad_index() {
        assert!(parse_off("OFF\n3 1 0\n0 0 0\n1 0 0\n0 1 0\n3 0 1 3\n").is_err());
    }
}
