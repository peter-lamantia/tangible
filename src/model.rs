pub(crate) mod gltf;
mod obj;
mod off;
mod ply;
mod three_mf;
use crate::{
    mesh::{read_limited, Mesh},
    stl,
};
use obj::parse_obj;
use std::{error::Error, path::Path};
use three_mf::load_3mf_document;
type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug)]
pub struct Plate {
    pub name: String,
    pub mesh: Mesh,
}

#[derive(Debug)]
pub struct ModelDocument {
    pub units: Option<&'static str>,
    pub mesh: Mesh,
    pub plates: Vec<Plate>,
}

pub(crate) fn load_document_with_context(
    path: impl AsRef<Path>,
    context: crate::loading::LoadContext,
) -> Result<ModelDocument> {
    let _scope = crate::mesh::loading_scope(context);
    load_document(path)
}

pub fn load_document(path: impl AsRef<Path>) -> Result<ModelDocument> {
    let path = path.as_ref();
    if path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("3mf"))
    {
        return load_3mf_document(path);
    }
    Ok(ModelDocument {
        units: None,
        mesh: load_model(path)?,
        plates: Vec::new(),
    })
}

pub fn load_model(path: impl AsRef<Path>) -> Result<Mesh> {
    let path = path.as_ref();
    match path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("stl") => stl::load_stl(path),
        Some("ply") => ply::load(path),
        Some("gltf" | "glb") => gltf::load(path),
        Some("off") => off::parse_off(std::str::from_utf8(&read_limited(path)?)?),
        Some("obj") => parse_obj(std::str::from_utf8(&read_limited(path)?)?),
        Some("3mf") => Ok(load_3mf_document(path)?.mesh),
        _ => Err(
            "Unsupported model format (expected .stl, .obj, .3mf, .off, .ply, .gltf, or .glb)"
                .into(),
        ),
    }
}

#[cfg(test)]
#[test]
#[ignore = "set TANGIBLE_BENCH_MODELS to paths separated by the platform path separator"]
fn benchmark_models() {
    let paths = std::env::var_os("TANGIBLE_BENCH_MODELS").expect("set TANGIBLE_BENCH_MODELS");
    println!("path\tload_ms\tvertex_bytes\ttexture_bytes\tanimation_ms");
    for path in std::env::split_paths(&paths) {
        let start = std::time::Instant::now();
        let document = load_document(&path).unwrap();
        let load_ms = start.elapsed().as_secs_f64() * 1000.;
        let mesh = &document.mesh;
        let start = std::time::Instant::now();
        if let Some(scene) = &mesh.animation {
            for frame in 0..60 {
                std::hint::black_box(scene.evaluate(frame as f32 / 60.).unwrap());
            }
        }
        let animation_ms = if mesh.animation.is_some() {
            start.elapsed().as_secs_f64() * 1000. / 60.
        } else {
            0.
        };
        println!(
            "{}\t{load_ms:.3}\t{}\t{}\t{animation_ms:.3}",
            path.display(),
            mesh.vertices.len() * std::mem::size_of::<crate::mesh::Vertex>(),
            mesh.textures.iter().map(|t| t.pixels.len()).sum::<usize>()
        );
    }
}
