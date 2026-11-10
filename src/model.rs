use crate::stl::{self, Mesh, Vertex};
use glam::Vec3;
use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};
use std::{collections::HashMap, error::Error, io::Read, path::Path};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug)]
pub struct Plate {
    pub name: String,
    pub mesh: Mesh,
}

#[derive(Debug)]
pub struct ModelDocument {
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
        Some("obj") => parse_obj(&std::fs::read_to_string(path)?),
        Some("3mf") => Ok(load_3mf_document(path)?.mesh),
        _ => Err("Unsupported model format (expected .stl, .obj, or .3mf)".into()),
    }
}

fn mesh_from_triangles(triangles: impl IntoIterator<Item = [[f32; 3]; 3]>) -> Result<Mesh> {
    let mut mesh = Mesh::empty();
    mesh.bounds_min = Vec3::splat(f32::INFINITY);
    mesh.bounds_max = Vec3::splat(f32::NEG_INFINITY);
    for triangle in triangles {
        let a = Vec3::from(triangle[0]);
        let b = Vec3::from(triangle[1]);
        let c = Vec3::from(triangle[2]);
        if ![a, b, c].iter().all(|p| p.is_finite()) {
            return Err("Non-finite vertex coordinate".into());
        }
        let normal = (b - a).cross(c - a).normalize_or_zero().to_array();
        for p in [a, b, c] {
            mesh.bounds_min = mesh.bounds_min.min(p);
            mesh.bounds_max = mesh.bounds_max.max(p);
            mesh.vertices.push(Vertex {
                position: p.to_array(),
                normal,
            });
        }
    }
    if mesh.vertices.is_empty() {
        return Err("Model contains no triangles".into());
    }
    Ok(mesh)
}

fn parse_obj(text: &str) -> Result<Mesh> {
    let mut points = Vec::<[f32; 3]>::new();
    let mut triangles = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
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
                points.push([p[0], p[1], p[2]]);
            }
            Some("f") => {
                let mut face = Vec::new();
                for part in parts {
                    let index: isize = part.split('/').next().unwrap_or("").parse()?;
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
                    face.push(points[index as usize]);
                }
                if face.len() < 3 {
                    return Err(format!(
                        "OBJ line {}: face has fewer than three vertices",
                        line_no + 1
                    )
                    .into());
                }
                for i in 1..face.len() - 1 {
                    triangles.push([face[0], face[i], face[i + 1]]);
                }
            }
            _ => {}
        }
    }
    mesh_from_triangles(triangles)
}

fn attr(element: &BytesStart<'_>, name: &[u8]) -> Result<Option<String>> {
    for attribute in element.attributes() {
        let attribute = attribute?;
        if attribute.key.as_ref().rsplit(|&b| b == b':').next() == Some(name) {
            return Ok(Some(String::from_utf8(attribute.value.into_owned())?));
        }
    }
    Ok(None)
}

fn required(element: &BytesStart<'_>, name: &[u8]) -> Result<String> {
    attr(element, name)?
        .ok_or_else(|| format!("3MF missing {} attribute", String::from_utf8_lossy(name)).into())
}

type Transform = [f32; 12];
const IDENTITY: Transform = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];

#[derive(Default)]
struct Object {
    points: Vec<[f32; 3]>,
    faces: Vec<[usize; 3]>,
    components: Vec<Reference>,
}

struct Reference {
    path: Option<String>,
    id: String,
    transform: Transform,
}

struct Model {
    objects: HashMap<String, Object>,
    items: Vec<Reference>,
}

fn transform_attr(element: &BytesStart<'_>) -> Result<Transform> {
    match attr(element, b"transform")? {
        None => Ok(IDENTITY),
        Some(value) => {
            let values: Vec<f32> = value
                .split_whitespace()
                .map(str::parse)
                .collect::<std::result::Result<_, _>>()?;
            values
                .try_into()
                .map_err(|_| "3MF transform must contain 12 numbers".into())
        }
    }
}

fn reference(element: &BytesStart<'_>) -> Result<Reference> {
    Ok(Reference {
        path: attr(element, b"path")?,
        id: required(element, b"objectid")?,
        transform: transform_attr(element)?,
    })
}

fn parse_model(xml: &str) -> Result<Model> {
    let mut reader = Reader::from_str(xml);
    let mut model = Model {
        objects: HashMap::new(),
        items: Vec::new(),
    };
    let mut current: Option<(String, Object)> = None;
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => match e.local_name().as_ref() {
                b"object" => current = Some((required(&e, b"id")?, Object::default())),
                b"vertex" => {
                    if let Some((_, object)) = &mut current {
                        object.points.push([
                            required(&e, b"x")?.parse()?,
                            required(&e, b"y")?.parse()?,
                            required(&e, b"z")?.parse()?,
                        ]);
                    }
                }
                b"triangle" => {
                    if let Some((_, object)) = &mut current {
                        object.faces.push([
                            required(&e, b"v1")?.parse()?,
                            required(&e, b"v2")?.parse()?,
                            required(&e, b"v3")?.parse()?,
                        ]);
                    }
                }
                b"component" => {
                    if let Some((_, object)) = &mut current {
                        object.components.push(reference(&e)?);
                    }
                }
                b"item" => model.items.push(reference(&e)?),
                _ => {}
            },
            Event::End(e) if e.local_name().as_ref() == b"object" => {
                if let Some((id, object)) = current.take() {
                    model.objects.insert(id, object);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(model)
}

fn apply_transform(m: &Transform, p: [f32; 3]) -> [f32; 3] {
    [
        m[0] * p[0] + m[3] * p[1] + m[6] * p[2] + m[9],
        m[1] * p[0] + m[4] * p[1] + m[7] * p[2] + m[10],
        m[2] * p[0] + m[5] * p[1] + m[8] * p[2] + m[11],
    ]
}

fn combine(a: &Transform, b: &Transform) -> Transform {
    let mut out = [0.0; 12];
    for axis in 0..3 {
        let mut p = [0.0; 3];
        p[axis] = 1.0;
        let q = apply_transform(a, apply_transform(b, p));
        let origin = apply_transform(a, apply_transform(b, [0.0; 3]));
        for row in 0..3 {
            out[axis * 3 + row] = q[row] - origin[row];
        }
    }
    out[9..12].copy_from_slice(&apply_transform(a, apply_transform(b, [0.0; 3])));
    out
}

fn resolve_path(current: &str, reference: Option<&str>) -> String {
    match reference {
        None => current.to_owned(),
        Some(path) if path.starts_with('/') => path.trim_start_matches('/').to_owned(),
        Some(path) => {
            let parent = current.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
            format!("{parent}/{path}")
        }
    }
}

fn collect_object(
    models: &HashMap<String, Model>,
    path: &str,
    id: &str,
    transform: &Transform,
    active: &mut Vec<(String, String)>,
    triangles: &mut Vec<[[f32; 3]; 3]>,
) -> Result<()> {
    let key = (path.to_owned(), id.to_owned());
    if active.contains(&key) {
        return Err("3MF component cycle".into());
    }
    let model = models
        .get(path)
        .ok_or_else(|| format!("3MF model part missing: {path}"))?;
    let object = model
        .objects
        .get(id)
        .ok_or_else(|| format!("3MF object {id} missing in {path}"))?;
    active.push(key);
    for face in &object.faces {
        let mut triangle = [[0.0; 3]; 3];
        for (i, &index) in face.iter().enumerate() {
            triangle[i] = apply_transform(
                transform,
                *object
                    .points
                    .get(index)
                    .ok_or("3MF triangle index out of range")?,
            );
        }
        triangles.push(triangle);
    }
    for component in &object.components {
        let child_path = resolve_path(path, component.path.as_deref());
        let child_transform = combine(transform, &component.transform);
        collect_object(
            models,
            &child_path,
            &component.id,
            &child_transform,
            active,
            triangles,
        )?;
    }
    active.pop();
    Ok(())
}

fn assemble_3mf(models: &HashMap<String, Model>, root: &str) -> Result<Mesh> {
    let model = models.get(root).ok_or("3MF root model missing")?;
    let mut triangles = Vec::new();
    for item in &model.items {
        let path = resolve_path(root, item.path.as_deref());
        collect_object(
            models,
            &path,
            &item.id,
            &item.transform,
            &mut Vec::new(),
            &mut triangles,
        )?;
    }
    mesh_from_triangles(triangles)
}

fn load_3mf_document(path: &Path) -> Result<ModelDocument> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    let names: Vec<String> = archive
        .file_names()
        .filter(|name| name.to_ascii_lowercase().ends_with(".model"))
        .map(str::to_owned)
        .collect();
    let root = names
        .iter()
        .find(|name| name.eq_ignore_ascii_case("3D/3dmodel.model"))
        .or_else(|| {
            names
                .iter()
                .find(|name| name.to_ascii_lowercase().starts_with("3d/"))
        })
        .ok_or("3MF archive contains no 3D model")?
        .clone();
    let mut models = HashMap::new();
    for name in names {
        let mut xml = String::new();
        archive.by_name(&name)?.read_to_string(&mut xml)?;
        models.insert(name, parse_model(&xml)?);
    }
    let all = assemble_3mf(&models, &root)?;
    let plates = if let Ok(mut settings) = archive.by_name("Metadata/model_settings.config") {
        let mut xml = String::new();
        settings.read_to_string(&mut xml)?;
        parse_plates(&xml, &models, &root)?
    } else {
        Vec::new()
    };
    let plates = if plates.is_empty() {
        vec![Plate {
            name: "Plate 1".to_owned(),
            mesh: all.clone(),
        }]
    } else {
        plates
    };
    Ok(ModelDocument { mesh: all, plates })
}

struct PlateSpec {
    name: String,
    instances: Vec<(String, usize)>,
}

fn parse_plates(xml: &str, models: &HashMap<String, Model>, root: &str) -> Result<Vec<Plate>> {
    let mut reader = Reader::from_str(xml);
    let mut specs = Vec::<PlateSpec>::new();
    let mut current: Option<PlateSpec> = None;
    let mut in_instance = false;
    let mut instance_id = None;
    let mut object_id = None;
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => match e.local_name().as_ref() {
                b"plate" => {
                    current = Some(PlateSpec {
                        name: format!("Plate {}", specs.len() + 1),
                        instances: Vec::new(),
                    })
                }
                b"model_instance" if current.is_some() => {
                    in_instance = true;
                    instance_id = None;
                    object_id = None;
                }
                b"metadata" if current.is_some() => {
                    let key = attr(&e, b"key")?.unwrap_or_default();
                    let value = attr(&e, b"value")?.unwrap_or_default();
                    match key.as_str() {
                        "plater_name" if !value.is_empty() => {
                            current.as_mut().unwrap().name = value
                        }
                        "object_id" if in_instance => object_id = Some(value),
                        "instance_id" if in_instance => instance_id = Some(value.parse::<usize>()?),
                        _ => {}
                    }
                }
                _ => {}
            },
            Event::End(e) => match e.local_name().as_ref() {
                b"model_instance" => {
                    if let (Some(plate), Some(id)) = (&mut current, object_id.take()) {
                        plate.instances.push((id, instance_id.take().unwrap_or(0)));
                    }
                    in_instance = false;
                }
                b"plate" => {
                    if let Some(plate) = current.take() {
                        specs.push(plate);
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    let root_model = models.get(root).ok_or("3MF root model missing")?;
    let mut plates = Vec::new();
    for spec in specs {
        let mut triangles = Vec::new();
        for (id, instance) in spec.instances {
            let item = root_model
                .items
                .iter()
                .filter(|item| item.id == id)
                .nth(instance)
                .ok_or_else(|| {
                    format!("3MF plate references missing instance {instance} of object {id}")
                })?;
            let path = resolve_path(root, item.path.as_deref());
            collect_object(
                models,
                &path,
                &item.id,
                &item.transform,
                &mut Vec::new(),
                &mut triangles,
            )?;
        }
        plates.push(Plate {
            name: spec.name,
            mesh: if triangles.is_empty() {
                Mesh::empty()
            } else {
                mesh_from_triangles(triangles)?
            },
        });
    }
    Ok(plates)
}

#[cfg(test)]
fn parse_3mf(xml: &str) -> Result<Mesh> {
    let mut models = HashMap::new();
    models.insert("3D/3dmodel.model".to_owned(), parse_model(xml)?);
    assemble_3mf(&models, "3D/3dmodel.model")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
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
    #[test]
    fn three_mf_build_transform() {
        let xml = r#"<model xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object></resources><build><item objectid="1" transform="1 0 0 0 1 0 0 0 1 5 6 7"/></build></model>"#;
        let mesh = parse_3mf(xml).unwrap();
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.bounds_min, Vec3::new(5.0, 6.0, 7.0));
        assert_eq!(mesh.bounds_max, Vec3::new(6.0, 7.0, 7.0));
    }
    #[test]
    fn three_mf_rejects_bad_index() {
        let xml = r#"<model><resources><object id="1"><mesh><triangle v1="0" v2="1" v3="2"/></mesh></object></resources><build><item objectid="1"/></build></model>"#;
        assert!(parse_3mf(xml).is_err());
    }

    #[test]
    fn three_mf_external_component() {
        let root = r#"<model><resources><object id="2"><components><component path="/3D/Objects/part.model" objectid="1" transform="1 0 0 0 1 0 0 0 1 2 0 0"/></components></object></resources><build><item objectid="2" transform="1 0 0 0 1 0 0 0 1 0 3 0"/></build></model>"#;
        let part = r#"<model><resources><object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object></resources></model>"#;
        let mut models = HashMap::new();
        models.insert("3D/3dmodel.model".to_owned(), parse_model(root).unwrap());
        models.insert(
            "3D/Objects/part.model".to_owned(),
            parse_model(part).unwrap(),
        );
        let mesh = assemble_3mf(&models, "3D/3dmodel.model").unwrap();
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.bounds_min, Vec3::new(2.0, 3.0, 0.0));
        assert_eq!(mesh.bounds_max, Vec3::new(3.0, 4.0, 0.0));
    }

    #[test]
    fn plate_metadata_selects_instances() {
        let root = r#"<model><resources><object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object></resources><build><item objectid="1"/><item objectid="1" transform="1 0 0 0 1 0 0 0 1 10 0 0"/></build></model>"#;
        let settings = r#"<config><plate><metadata key="plater_name" value="First"/><model_instance><metadata key="object_id" value="1"/><metadata key="instance_id" value="0"/></model_instance></plate><plate><model_instance><metadata key="object_id" value="1"/><metadata key="instance_id" value="1"/></model_instance></plate></config>"#;
        let mut models = HashMap::new();
        models.insert("3D/3dmodel.model".to_owned(), parse_model(root).unwrap());
        let plates = parse_plates(settings, &models, "3D/3dmodel.model").unwrap();
        assert_eq!(plates.len(), 2);
        assert_eq!(plates[0].name, "First");
        assert_eq!(plates[0].mesh.bounds_min, Vec3::ZERO);
        assert_eq!(plates[1].mesh.bounds_min, Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(
            assemble_3mf(&models, "3D/3dmodel.model")
                .unwrap()
                .vertices
                .len(),
            6
        );
    }

    #[test]
    fn loads_3mf_archive() {
        let path = std::env::temp_dir().join(format!("tangible-test-{}.3mf", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file("3D/3dmodel.model", zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(br#"<model><resources><object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object></resources><build><item objectid="1"/></build></model>"#).unwrap();
        archive
            .start_file(
                "Metadata/model_settings.config",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive.write_all(br#"<config><plate><metadata key="plater_name" value="Test plate"/><model_instance><metadata key="object_id" value="1"/><metadata key="instance_id" value="0"/></model_instance></plate></config>"#).unwrap();
        archive.finish().unwrap();
        let document = load_document(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(document.mesh.vertices.len(), 3);
        assert_eq!(document.mesh.bounds_max, Vec3::new(1.0, 1.0, 0.0));
        assert_eq!(document.plates.len(), 1);
        assert_eq!(document.plates[0].name, "Test plate");
        assert_eq!(document.plates[0].mesh.vertices.len(), 3);
    }
}
