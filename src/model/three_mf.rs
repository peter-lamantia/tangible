use super::{ModelDocument, Plate, Result};
use crate::mesh::{Mesh, MAX_TRIANGLES};
use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};
use std::{collections::HashMap, io::Read, path::Path};

fn attr(element: &BytesStart<'_>, name: &[u8]) -> Result<Option<String>> {
    for attribute in element.attributes() {
        let attribute = attribute?;
        if attribute.key.as_ref().rsplit(|&b| b == b':').next() == Some(name) {
            return Ok(Some(
                attribute
                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)?
                    .into_owned(),
            ));
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
    unit: f32,
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
            if !values.iter().all(|v| v.is_finite()) {
                return Err("Non-finite 3MF transform".into());
            }
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
    validate_xml(xml)?;
    let mut reader = Reader::from_str(xml);
    let mut model = Model {
        unit: 1.0,
        objects: HashMap::new(),
        items: Vec::new(),
    };
    let mut current: Option<(String, Object)> = None;
    loop {
        crate::mesh::check_cancelled()?;
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => match e.local_name().as_ref() {
                b"model" => {
                    model.unit = match attr(&e, b"unit")?.as_deref().unwrap_or("millimeter") {
                        "micron" => 0.001,
                        "millimeter" => 1.0,
                        "centimeter" => 10.0,
                        "inch" => 25.4,
                        "foot" => 304.8,
                        "meter" => 1000.0,
                        _ => return Err("Unsupported 3MF unit".into()),
                    };
                }
                b"object" => current = Some((required(&e, b"id")?, Object::default())),
                b"vertex" => {
                    if let Some((_, object)) = &mut current {
                        if object.points.len() >= MAX_TRIANGLES * 3 {
                            return Err("3MF exceeds vertex limit".into());
                        }
                        let point: [f32; 3] = [
                            required(&e, b"x")?.parse()?,
                            required(&e, b"y")?.parse()?,
                            required(&e, b"z")?.parse()?,
                        ];
                        if !point.iter().all(|v| v.is_finite()) {
                            return Err("Non-finite 3MF vertex".into());
                        }
                        object.points.push(point);
                    }
                }
                b"triangle" => {
                    if let Some((_, object)) = &mut current {
                        if object.faces.len() >= MAX_TRIANGLES {
                            return Err("3MF exceeds triangle limit".into());
                        }
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
    for column in 0..3 {
        for row in 0..3 {
            out[column * 3 + row] = (0..3)
                .map(|k| a[k * 3 + row] as f64 * b[column * 3 + k] as f64)
                .sum::<f64>() as f32;
        }
    }
    for row in 0..3 {
        out[9 + row] = ((0..3)
            .map(|k| a[k * 3 + row] as f64 * b[9 + k] as f64)
            .sum::<f64>()
            + a[9 + row] as f64) as f32;
    }
    out
}

fn scale(factor: f32) -> Transform {
    let mut m = IDENTITY;
    m[0] = factor;
    m[4] = factor;
    m[8] = factor;
    m
}

fn resolve_path(current: &str, reference: Option<&str>) -> Result<String> {
    let Some(path) = reference else {
        return Ok(current.to_owned());
    };
    if path.contains(['\\', ':', '?', '#']) {
        return Err("Invalid 3MF package path".into());
    }
    let joined = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!(
            "{}/{}",
            current.rsplit_once('/').map(|(p, _)| p).unwrap_or(""),
            path
        )
    };
    let mut segments = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    return Err("3MF path escapes package".into());
                }
            }
            part => segments.push(part),
        }
    }
    if segments.is_empty() {
        return Err("Empty 3MF package path".into());
    }
    Ok(segments.join("/"))
}

fn reference_transform(
    models: &HashMap<String, Model>,
    current: &str,
    reference: &Reference,
) -> Result<(String, Transform)> {
    let path = resolve_path(current, reference.path.as_deref())?;
    let source = models.get(current).ok_or("Missing 3MF source part")?;
    let target = models.get(&path).ok_or("Missing 3MF target part")?;
    Ok((
        path,
        combine(&reference.transform, &scale(target.unit / source.unit)),
    ))
}

#[derive(Default)]
struct Expansion {
    active: Vec<(String, String)>,
    visits: usize,
}

fn collect_object(
    models: &HashMap<String, Model>,
    path: &str,
    id: &str,
    transform: &Transform,
    budget: &mut Expansion,
    triangles: &mut Mesh,
) -> Result<()> {
    let key = (path.to_owned(), id.to_owned());
    budget.visits += 1;
    if budget.visits > 100_000 || budget.active.len() >= 64 {
        return Err("3MF component expansion limit exceeded".into());
    }
    if budget.active.contains(&key) {
        return Err("3MF component cycle".into());
    }
    let model = models
        .get(path)
        .ok_or_else(|| format!("3MF model part missing: {path}"))?;
    let object = model
        .objects
        .get(id)
        .ok_or_else(|| format!("3MF object {id} missing in {path}"))?;
    budget.active.push(key);
    for face in &object.faces {
        if (triangles.vertices.len() / 3).is_multiple_of(1024) {
            crate::mesh::check_cancelled()?;
        }
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
        if (triangles.vertices.len() / 3) >= MAX_TRIANGLES {
            return Err("3MF exceeds expanded triangle limit".into());
        }
        triangles.push_triangle(triangle)?;
    }
    for component in &object.components {
        crate::mesh::check_cancelled()?;
        let (child_path, local_transform) = reference_transform(models, path, component)?;
        let child_transform = combine(transform, &local_transform);
        collect_object(
            models,
            &child_path,
            &component.id,
            &child_transform,
            budget,
            triangles,
        )?;
    }
    budget.active.pop();
    Ok(())
}

fn assemble_3mf(models: &HashMap<String, Model>, root: &str) -> Result<Mesh> {
    let model = models.get(root).ok_or("3MF root model missing")?;
    let mut triangles = Mesh::triangle_builder(0);
    let mut budget = Expansion::default();
    for item in &model.items {
        let (path, transform) = reference_transform(models, root, item)?;
        collect_object(
            models,
            &path,
            &item.id,
            &combine(&scale(model.unit), &transform),
            &mut budget,
            &mut triangles,
        )?;
    }
    triangles.finish()
}

const MAX_XML_BYTES: u64 = 256 * 1024 * 1024;
const MODEL_RELATIONSHIP: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";

fn read_xml(reader: impl Read) -> Result<String> {
    read_xml_with_limit(reader, MAX_XML_BYTES)
}

fn read_xml_with_limit(reader: impl Read, limit: u64) -> Result<String> {
    let mut bytes = Vec::new();
    let mut reader = reader.take(limit + 1);
    let mut chunk = [0; 65536];
    loop {
        crate::mesh::check_cancelled()?;
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    let xml = String::from_utf8(bytes)?;
    if xml.len() as u64 > limit {
        return Err("3MF XML part exceeds 256 MiB limit".into());
    }
    validate_xml(&xml)?;
    Ok(xml)
}

fn validate_xml(xml: &str) -> Result<()> {
    let mut reader = Reader::from_str(xml);
    let mut depth = 0usize;
    let mut roots = 0;
    loop {
        crate::mesh::check_cancelled()?;
        match reader.read_event()? {
            Event::Start(_) => {
                if depth == 0 {
                    roots += 1;
                }
                depth += 1;
                if depth > 128 {
                    return Err("3MF XML nesting limit exceeded".into());
                }
            }
            Event::Empty(_) if depth == 0 => roots += 1,
            Event::End(_) => depth = depth.checked_sub(1).ok_or("Unbalanced XML")?,
            Event::DocType(_) => return Err("XML document types are unsupported".into()),
            Event::Eof => break,
            _ => {}
        }
    }
    if depth != 0 || roots != 1 {
        return Err("Incomplete or invalid 3MF XML document".into());
    }
    Ok(())
}

fn relationship_root(xml: &str) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    let mut root = None;
    loop {
        crate::mesh::check_cancelled()?;
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"Relationship" => {
                if attr(&e, b"Type")?.as_deref() == Some(MODEL_RELATIONSHIP) {
                    if root.is_some() {
                        return Err("Multiple 3MF start parts".into());
                    }
                    if attr(&e, b"TargetMode")?
                        .as_deref()
                        .is_some_and(|m| m != "Internal")
                    {
                        return Err("External 3MF start part is unsupported".into());
                    }
                    root = Some(resolve_path("", Some(&required(&e, b"Target")?))?);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    root.ok_or_else(|| "3MF package relationships contain no model start part".into())
}

pub(super) fn load_3mf_document(path: &Path) -> Result<ModelDocument> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > crate::mesh::MAX_INPUT_BYTES {
        return Err("3MF archive exceeds 512 MiB limit".into());
    }
    let mut archive = zip::ZipArchive::new(file)?;
    if archive.len() > 4096 {
        return Err("3MF archive exceeds 4096 entry limit".into());
    }
    let mut total_bytes = 0u64;
    for i in 0..archive.len() {
        crate::mesh::check_cancelled()?;
        total_bytes = total_bytes
            .checked_add(archive.by_index(i)?.size())
            .ok_or("3MF archive size overflow")?;
        if total_bytes > crate::mesh::MAX_INPUT_BYTES {
            return Err("3MF archive exceeds 512 MiB decompressed limit".into());
        }
    }
    let names: Vec<String> = archive
        .file_names()
        .filter(|name| name.to_ascii_lowercase().ends_with(".model"))
        .map(str::to_owned)
        .collect();
    let root = match archive.by_name("_rels/.rels") {
        Ok(part) => relationship_root(&read_xml(part)?)?,
        // Compatibility with older slicer exports that omit package relationships.
        Err(zip::result::ZipError::FileNotFound) => names
            .iter()
            .find(|name| name.eq_ignore_ascii_case("3D/3dmodel.model"))
            .cloned()
            .ok_or("3MF package has no start-part relationship")?,
        Err(error) => return Err(error.into()),
    };
    let mut models = HashMap::new();
    for name in names {
        crate::mesh::check_cancelled()?;
        let xml = read_xml(archive.by_name(&name)?)?;
        models.insert(name, parse_model(&xml)?);
    }
    let all = assemble_3mf(&models, &root)?;
    let plates = match archive.by_name("Metadata/model_settings.config") {
        Ok(settings) => parse_plates(&read_xml(settings)?, &models, &root)?,
        Err(zip::result::ZipError::FileNotFound) => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    let plates = if plates.is_empty() {
        vec![Plate {
            name: "Plate 1".to_owned(),
            mesh: all.clone(),
        }]
    } else {
        plates
    };
    Ok(ModelDocument {
        units: Some("mm"),
        mesh: all,
        plates,
    })
}

struct PlateSpec {
    name: String,
    instances: Vec<(String, usize)>,
}

fn parse_plates(xml: &str, models: &HashMap<String, Model>, root: &str) -> Result<Vec<Plate>> {
    validate_xml(xml)?;
    let mut reader = Reader::from_str(xml);
    let mut specs = Vec::<PlateSpec>::new();
    let mut current: Option<PlateSpec> = None;
    let mut in_instance = false;
    let mut instance_id = None;
    let mut object_id = None;
    loop {
        crate::mesh::check_cancelled()?;
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
                        if specs.len() >= 4096 {
                            return Err("3MF exceeds 4096 plate limit".into());
                        }
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
    let mut budget = Expansion::default();
    let mut total_triangles = 0;
    for spec in specs {
        crate::mesh::check_cancelled()?;
        let mut triangles = Mesh::triangle_builder(0);
        for (id, instance) in spec.instances {
            let item = root_model
                .items
                .iter()
                .filter(|item| item.id == id)
                .nth(instance)
                .ok_or_else(|| {
                    format!("3MF plate references missing instance {instance} of object {id}")
                })?;
            let (path, transform) = reference_transform(models, root, item)?;
            collect_object(
                models,
                &path,
                &item.id,
                &combine(&scale(root_model.unit), &transform),
                &mut budget,
                &mut triangles,
            )?;
        }
        total_triangles += triangles.vertices.len() / 3;
        if total_triangles > MAX_TRIANGLES {
            return Err("3MF plates exceed expanded triangle limit".into());
        }
        plates.push(Plate {
            name: spec.name,
            mesh: if triangles.vertices.is_empty() {
                Mesh::empty()
            } else {
                triangles.finish()?
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
    use crate::model::load_document;
    use glam::Vec3;
    use std::io::Write;
    const TRIANGLE: &str = r#"<model><resources><object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object></resources><build><item objectid="1" transform="1 0 0 0 1 0 0 0 1 2 0 0"/></build></model>"#;

    #[test]
    fn rejects_excessive_declared_decompressed_size_before_reading() {
        let path = std::env::temp_dir().join(format!(
            "tangible-decompressed-limit-{}.3mf",
            std::process::id()
        ));
        let cursor = std::io::Cursor::new(Vec::new());
        let mut archive = zip::ZipWriter::new(cursor);
        archive
            .start_file("large.model", zip::write::SimpleFileOptions::default())
            .unwrap();
        let mut bytes = archive.finish().unwrap().into_inner();
        let central = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        bytes[central + 24..central + 28]
            .copy_from_slice(&((crate::mesh::MAX_INPUT_BYTES + 1) as u32).to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        let error = load_document(&path).unwrap_err();
        std::fs::remove_file(path).unwrap();
        assert!(error.to_string().contains("decompressed limit"));
    }

    #[test]
    fn xml_and_component_work_limits_are_enforced() {
        assert!(read_xml_with_limit("<model/>".as_bytes(), 8).is_ok());
        assert!(read_xml_with_limit("<model/> ".as_bytes(), 8).is_err());
        let mut models = HashMap::new();
        models.insert("3D/3dmodel.model".into(), parse_model(TRIANGLE).unwrap());
        let mut budget = Expansion {
            active: Vec::new(),
            visits: 100_000,
        };
        let error = collect_object(
            &models,
            "3D/3dmodel.model",
            "1",
            &IDENTITY,
            &mut budget,
            &mut Mesh::triangle_builder(0),
        )
        .unwrap_err();
        assert!(error.to_string().contains("limit"));
    }

    #[test]
    fn rejects_archives_with_too_many_entries() {
        let path =
            std::env::temp_dir().join(format!("tangible-many-entries-{}.3mf", std::process::id()));
        let mut archive = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for i in 0..4097 {
            archive
                .start_file(
                    format!("entry-{i}"),
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
        }
        archive.finish().unwrap();
        let error = load_document(&path).unwrap_err();
        std::fs::remove_file(path).unwrap();
        assert!(error.to_string().contains("entry limit"));
    }

    #[test]
    fn three_mf_units_scale_geometry_and_translation() {
        for (unit, factor) in [
            ("micron", 0.001),
            ("millimeter", 1.0),
            ("centimeter", 10.0),
            ("inch", 25.4),
            ("foot", 304.8),
            ("meter", 1000.0),
        ] {
            let xml = TRIANGLE.replacen("<model>", &format!("<model unit=\"{unit}\">"), 1);
            let mesh = parse_3mf(&xml).unwrap();
            assert!((mesh.bounds_min.x - 2.0 * factor).abs() < 1e-4);
            assert!((mesh.bounds_max.x - 3.0 * factor).abs() < 1e-4);
        }
        assert!(parse_3mf(&TRIANGLE.replace("<model>", "<model unit=\"yard\"> ")).is_err());
    }
    #[test]
    fn three_mf_cross_part_units_and_relative_paths() {
        let root = r#"<model unit="centimeter"><resources><object id="2"><components><component path="./Objects/../part.model" objectid="1" transform="1 0 0 0 1 0 0 0 1 2 0 0"/></components></object></resources><build><item objectid="2"/></build></model>"#;
        let mut models = HashMap::new();
        models.insert("3D/root.model".into(), parse_model(root).unwrap());
        models.insert(
            "3D/part.model".into(),
            parse_model(&TRIANGLE.replace("<model>", "<model unit=\"inch\"> ")).unwrap(),
        );
        let mesh = assemble_3mf(&models, "3D/root.model").unwrap();
        assert!((mesh.bounds_min.x - 20.0).abs() < 1e-4);
        assert!((mesh.bounds_max.x - 45.4).abs() < 1e-4);
        assert!(resolve_path("3D/root.model", Some("../../outside.model")).is_err());
    }
    #[test]
    fn three_mf_rejects_cycles_deep_trees_and_bad_xml() {
        let cycle = r#"<model><resources><object id="1"><components><component objectid="1"/></components></object></resources><build><item objectid="1"/></build></model>"#;
        assert!(parse_3mf(cycle).unwrap_err().to_string().contains("cycle"));
        let mut xml = "<model><resources>".to_owned();
        for id in 1..=65 {
            xml.push_str(&format!("<object id=\"{id}\"><components><component objectid=\"{}\"/></components></object>",id+1));
        }
        xml.push_str(
            "<object id=\"66\"></object></resources><build><item objectid=\"1\"/></build></model>",
        );
        assert!(parse_3mf(&xml).unwrap_err().to_string().contains("limit"));
        for xml in [
            "<model>",
            "<model></wrong>",
            "<model/><model/>",
            "<!DOCTYPE model><model/>",
        ] {
            assert!(parse_model(xml).is_err());
        }
        assert!(parse_3mf(&TRIANGLE.replace("x=\"1\"", "x=\"NaN\"")).is_err());
        assert!(parse_3mf(&TRIANGLE.replace("1 0 0 0 1 0", "NaN 0 0 0 1 0")).is_err());
    }
    #[test]
    fn archive_start_relationship_overrides_conventional_filename() {
        let path =
            std::env::temp_dir().join(format!("tangible-relationships-{}.3mf", std::process::id()));
        let mut archive = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for (name, content) in [
            (
                "_rels/.rels",
                format!(
                    r#"<Relationships><Relationship Id="root" Type="{MODEL_RELATIONSHIP}" Target="/Custom/scene.model"/></Relationships>"#
                ),
            ),
            (
                "Custom/scene.model",
                TRIANGLE.replace("<model>", "<model unit=\"inch\">"),
            ),
            ("3D/3dmodel.model", TRIANGLE.to_owned()),
        ] {
            archive
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(content.as_bytes()).unwrap();
        }
        archive.finish().unwrap();
        let document = load_document(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!((document.mesh.bounds_max.x - 76.2).abs() < 1e-4);
    }
    #[test]
    fn package_relationships_reject_external_or_missing_start_parts() {
        assert!(relationship_root("<Relationships/>").is_err());
        let xml = format!(
            r#"<Relationships><Relationship Type="{MODEL_RELATIONSHIP}" TargetMode="External" Target="https://example.com/model"/></Relationships>"#
        );
        assert!(relationship_root(&xml).is_err());
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
