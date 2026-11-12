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
