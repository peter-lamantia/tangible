//! Normalize static glTF 1.0 assets before passing them to the validated 2.0 reader.
use crate::model::Result;
use gltf::json::Value;
use std::collections::BTreeMap;

type Ids = BTreeMap<String, usize>;

fn object(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Object(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn collection(root: &Value, key: &str) -> Result<(Ids, Vec<Value>)> {
    let Some(value) = root.get(key) else {
        return Ok((Ids::new(), Vec::new()));
    };
    let entries = value.as_object().ok_or("Invalid glTF 1.0 collection")?;
    Ok((
        entries
            .keys()
            .enumerate()
            .map(|(i, k)| (k.clone(), i))
            .collect(),
        entries.values().cloned().collect(),
    ))
}
fn reference(value: &Value, ids: &Ids) -> Result<Value> {
    let id = value
        .as_str()
        .ok_or("Invalid glTF 1.0 resource reference")?;
    Ok((*ids
        .get(id)
        .ok_or_else(|| format!("Missing glTF 1.0 resource {id:?}"))?)
    .into())
}
fn remap(value: &mut Value, key: &str, ids: &Ids) -> Result<()> {
    if let Some(old) = value.get(key) {
        value[key] = reference(old, ids)?;
    }
    Ok(())
}
fn remap_array(value: &mut Value, key: &str, ids: &Ids) -> Result<()> {
    if let Some(old) = value.get(key) {
        value[key] = Value::Array(
            old.as_array()
                .ok_or("Invalid glTF 1.0 reference array")?
                .iter()
                .map(|v| reference(v, ids))
                .collect::<Result<_>>()?,
        );
    }
    Ok(())
}

pub(super) fn parse(bytes: &[u8]) -> Result<gltf::Gltf> {
    let (root, blob): (Value, _) = if bytes.starts_with(b"glTF\x01\0\0\0") {
        let word = |offset| {
            bytes
                .get(offset..offset + 4)
                .map(|v| u32::from_le_bytes(v.try_into().unwrap()) as usize)
                .ok_or("Truncated GLB 1.0 header")
        };
        let length = word(8)?;
        let json_length = word(12)?;
        if length != bytes.len() || word(16)? != 0 || json_length == 0 {
            return Err("Invalid GLB 1.0 header".into());
        }
        let end = 20usize
            .checked_add(json_length)
            .filter(|end| *end <= bytes.len())
            .ok_or("Truncated GLB 1.0 JSON")?;
        (
            gltf::json::deserialize::from_slice(&bytes[20..end])?,
            Some(bytes[end..].to_vec()),
        )
    } else {
        return match gltf::Gltf::from_slice(bytes) {
            Ok(source) => Ok(source),
            Err(error) => {
                // JSON glTF 1.0 uses the same named-resource representation.
                let root: Value = match gltf::json::deserialize::from_slice(bytes) {
                    Ok(root) => root,
                    Err(_) => return Err(error.into()),
                };
                if root["asset"]["version"] != "1.0" {
                    return Err(error.into());
                }
                normalize(root, None)
            }
        };
    };
    normalize(root, blob)
}

fn normalize(root: Value, blob: Option<Vec<u8>>) -> Result<gltf::Gltf> {
    if root["asset"]["version"] != "1.0" {
        return Err("Unsupported legacy glTF asset version (expected 1.0)".into());
    }
    if let Some(required) = root.get("extensionsRequired") {
        for extension in required
            .as_array()
            .ok_or("Invalid legacy required extensions")?
        {
            if !matches!(
                extension.as_str(),
                Some("KHR_binary_glTF" | "KHR_materials_common")
            ) {
                return Err(format!("Unsupported required glTF 1.0 extension: {extension}").into());
            }
        }
    }
    for key in ["animations", "skins"] {
        if !collection(&root, key)?.1.is_empty() {
            return Err(
                "Animated or skinned glTF 1.0 assets require conversion to glTF 2.0".into(),
            );
        }
    }
    let (buffer_ids, mut buffers) = collection(&root, "buffers")?;
    let (view_ids, mut views) = collection(&root, "bufferViews")?;
    let (accessor_ids, mut accessors) = collection(&root, "accessors")?;
    let (image_ids, mut images) = collection(&root, "images")?;
    let (sampler_ids, samplers) = collection(&root, "samplers")?;
    let (texture_ids, mut textures) = collection(&root, "textures")?;
    let (material_ids, materials) = collection(&root, "materials")?;
    let (mesh_ids, mut meshes) = collection(&root, "meshes")?;
    let (node_ids, mut nodes) = collection(&root, "nodes")?;
    let (scene_ids, mut scenes) = collection(&root, "scenes")?;
    for (id, index) in &buffer_ids {
        let buffer = buffers[*index]
            .as_object_mut()
            .ok_or("Invalid glTF 1.0 buffer")?;
        buffer.remove("type");
        if id == "binary_glTF" && blob.is_some() {
            buffer.remove("uri");
        }
    }
    for view in &mut views {
        remap(view, "buffer", &buffer_ids)?;
    }
    for accessor in &mut accessors {
        remap(accessor, "bufferView", &view_ids)?;
        let stride = accessor
            .as_object_mut()
            .ok_or("Invalid glTF 1.0 accessor")?
            .remove("byteStride");
        if let Some(stride) = stride.filter(|v| v.as_u64().is_some_and(|v| v > 0)) {
            let index = accessor["bufferView"]
                .as_u64()
                .ok_or("Missing glTF 1.0 accessor buffer view")? as usize;
            let mut view = views[index].clone();
            view["byteStride"] = stride;
            accessor["bufferView"] = views.len().into();
            views.push(view);
        }
    }
    for image in &mut images {
        if let Some(binary) = image["extensions"]["KHR_binary_glTF"].as_object() {
            let view = reference(
                binary
                    .get("bufferView")
                    .ok_or("Missing legacy image buffer view")?,
                &view_ids,
            )?;
            let mime = binary
                .get("mimeType")
                .ok_or("Missing legacy image MIME type")?
                .clone();
            let image = image.as_object_mut().ok_or("Invalid legacy image")?;
            image.remove("uri");
            image.remove("extensions");
            image.insert("bufferView".into(), view);
            image.insert("mimeType".into(), mime);
        }
    }
    for texture in &mut textures {
        remap(texture, "source", &image_ids)?;
        remap(texture, "sampler", &sampler_ids)?;
        let texture = texture.as_object_mut().ok_or("Invalid legacy texture")?;
        for key in ["format", "internalFormat", "target", "type"] {
            texture.remove(key);
        }
    }
    let materials: Vec<_> = materials
        .iter()
        .map(|material| -> Result<Value> {
            let values = material
                .get("values")
                .or_else(|| material["extensions"]["KHR_materials_common"].get("values"));
            let diffuse = values.and_then(|v| v.get("diffuse"));
            let mut pbr = object([("metallicFactor", 0.into()), ("roughnessFactor", 1.into())]);
            if let Some(diffuse) = diffuse {
                if diffuse.is_string() {
                    pbr["baseColorTexture"] =
                        object([("index", reference(diffuse, &texture_ids)?)]);
                } else if let Some(color) = diffuse.as_array() {
                    if color.len() != 3 && color.len() != 4 {
                        return Err("Invalid legacy diffuse color".into());
                    }
                    let mut color = color.clone();
                    if color.len() == 3 {
                        color.push(1.into());
                    }
                    if let Some(alpha) = values.and_then(|v| v["transparency"].as_f64()) {
                        color[3] = (color[3].as_f64().unwrap_or(1.) * alpha)
                            .clamp(0., 1.)
                            .into();
                    }
                    pbr["baseColorFactor"] = color.into();
                }
            }
            if let Some(shininess) = values.and_then(|v| v["shininess"].as_f64()) {
                pbr["roughnessFactor"] = (2. / (shininess.max(0.) + 2.)).sqrt().into();
            }
            let mut result = object([("pbrMetallicRoughness", pbr), ("doubleSided", true.into())]);
            if let Some(name) = material.get("name") {
                result["name"] = name.clone();
            }
            let alpha = result["pbrMetallicRoughness"]["baseColorFactor"][3]
                .as_f64()
                .unwrap_or(1.);
            if alpha < 1. {
                result["alphaMode"] = "BLEND".into();
            }
            if let Some(emission) = values
                .and_then(|v| v["emission"].as_array())
                .filter(|v| v.len() >= 3)
            {
                result["emissiveFactor"] = emission[..3].to_vec().into();
            }
            Ok(result)
        })
        .collect::<Result<_>>()?;
    for mesh in &mut meshes {
        for primitive in mesh["primitives"]
            .as_array_mut()
            .ok_or("Missing legacy mesh primitives")?
        {
            remap(primitive, "indices", &accessor_ids)?;
            remap(primitive, "material", &material_ids)?;
            for attribute in primitive["attributes"]
                .as_object_mut()
                .ok_or("Missing legacy vertex attributes")?
                .values_mut()
            {
                *attribute = reference(attribute, &accessor_ids)?;
            }
        }
    }
    for node in &mut nodes {
        remap_array(node, "children", &node_ids)?;
        let source_meshes = node
            .as_object_mut()
            .ok_or("Invalid legacy node")?
            .remove("meshes");
        if let Some(source_meshes) = source_meshes {
            let mut primitives = Vec::new();
            for id in source_meshes
                .as_array()
                .ok_or("Invalid legacy node meshes")?
            {
                let index = reference(id, &mesh_ids)?.as_u64().unwrap() as usize;
                primitives.extend(
                    meshes[index]["primitives"]
                        .as_array()
                        .ok_or("Missing legacy primitives")?
                        .iter()
                        .cloned(),
                );
            }
            if !primitives.is_empty() {
                node["mesh"] = meshes.len().into();
                meshes.push(object([("primitives", primitives.into())]));
            }
        }
        node.as_object_mut().unwrap().remove("camera");
    }
    for scene in &mut scenes {
        remap_array(scene, "nodes", &node_ids)?;
    }
    let mut converted = object([("asset", object([("version", "2.0".into())]))]);
    for (key, values) in [
        ("buffers", buffers),
        ("bufferViews", views),
        ("accessors", accessors),
        ("images", images),
        ("samplers", samplers),
        ("textures", textures),
        ("materials", materials),
        ("meshes", meshes),
        ("nodes", nodes),
        ("scenes", scenes),
    ] {
        if !values.is_empty() {
            converted[key] = values.into();
        }
    }
    if let Some(scene) = root.get("scene") {
        converted["scene"] = reference(scene, &scene_ids)?;
    }
    let bytes = gltf::json::serialize::to_vec(&converted)?;
    let mut source = gltf::Gltf::from_slice(&bytes)?;
    source.blob = blob;
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle() -> (Value, Vec<u8>) {
        let root = gltf::json::deserialize::from_str(r#"{
            "asset":{"version":"1.0"},
            "buffers":{"binary_glTF":{"byteLength":36,"uri":"data:,","type":"arraybuffer"}},
            "bufferViews":{"vertices":{"buffer":"binary_glTF","byteLength":36}},
            "accessors":{"positions":{"bufferView":"vertices","byteStride":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},
            "materials":{"red":{"values":{"diffuse":[1,0,0,1],"shininess":128}}},
            "meshes":{"triangle":{"primitives":[{"attributes":{"POSITION":"positions"},"material":"red","mode":4}]}},
            "nodes":{"root":{"meshes":["triangle"],"translation":[2,0,0]}},
            "scenes":{"main":{"nodes":["root"]}},"scene":"main"
        }"#).unwrap();
        let body = [0f32, 0., 0., 1., 0., 0., 0., 1., 0.]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        (root, body)
    }
    fn binary(root: &Value, body: &[u8]) -> Vec<u8> {
        let mut json = gltf::json::serialize::to_vec(root).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut bytes = Vec::new();
        bytes.extend(b"glTF");
        for word in [
            1,
            (20 + json.len() + body.len()) as u32,
            json.len() as u32,
            0,
        ] {
            bytes.extend(word.to_le_bytes());
        }
        bytes.extend(json);
        bytes.extend(body);
        bytes
    }

    #[test]
    fn imports_named_resources_transforms_and_legacy_materials() {
        let (root, body) = triangle();
        let bytes = binary(&root, &body);
        let dir = std::env::temp_dir().join(format!("tangible-legacy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("triangle.glb");
        std::fs::write(&path, &bytes).unwrap();
        let mesh = crate::model::load_model(&path).unwrap();
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.vertices[0].position, [2., 0., 0.]);
        assert_eq!(mesh.vertices[0].color, [1., 0., 0., 1.]);
        assert_eq!(mesh.vertices[0].surface[0], 0.);
        // The v1 binary image extension uses named bufferViews, not v2 indices.
        let mut root = root;
        let mut body = body;
        let image = include_bytes!("../../../assets/tangible-ui.png");
        root["buffers"]["binary_glTF"]["byteLength"] = (body.len() + image.len()).into();
        root["bufferViews"]["image"] = object([
            ("buffer", "binary_glTF".into()),
            ("byteOffset", 36.into()),
            ("byteLength", image.len().into()),
        ]);
        root["images"] = object([(
            "color",
            object([
                ("uri", "data:,".into()),
                (
                    "extensions",
                    object([(
                        "KHR_binary_glTF",
                        object([
                            ("bufferView", "image".into()),
                            ("mimeType", "image/png".into()),
                        ]),
                    )]),
                ),
            ]),
        )]);
        root["textures"] = object([("color", object([("source", "color".into())]))]);
        root["materials"]["red"]["values"]["diffuse"] = "color".into();
        body.extend(image);
        std::fs::write(&path, binary(&root, &body)).unwrap();
        let mesh = crate::model::load_model(&path).unwrap();
        assert_eq!(mesh.batches[0].texture, Some(0));
        assert_eq!(mesh.textures.len(), 1);
        assert!(mesh.textures[0].width > 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn imports_json_legacy_documents_and_accessor_stride() {
        let (mut root, _) = triangle();
        root["buffers"]["binary_glTF"]["uri"] = "mesh.bin".into();
        root["accessors"]["positions"]["byteStride"] = 12.into();
        let source = parse(&gltf::json::serialize::to_vec(&root).unwrap()).unwrap();
        assert_eq!(
            source
                .document
                .accessors()
                .next()
                .unwrap()
                .view()
                .unwrap()
                .stride(),
            Some(12)
        );
        assert!(source.blob.is_none());
        assert_eq!(
            source
                .document
                .nodes()
                .next()
                .unwrap()
                .mesh()
                .unwrap()
                .primitives()
                .len(),
            1
        );
    }

    #[test]
    fn rejects_bad_headers_references_and_unsupported_animation() {
        let (mut root, body) = triangle();
        let bytes = binary(&root, &body);
        for length in [8, 12, 19, bytes.len() - 1] {
            assert!(parse(&bytes[..length]).is_err());
        }
        root["nodes"]["root"]["meshes"] = vec![Value::from("missing")].into();
        assert!(parse(&binary(&root, &body))
            .unwrap_err()
            .to_string()
            .contains("Missing glTF 1.0 resource"));
        root["animations"] = object([("animation", object([]))]);
        assert!(parse(&binary(&root, &body))
            .unwrap_err()
            .to_string()
            .contains("require conversion to glTF 2.0"));
    }

    #[test]
    #[ignore = "set TANGIBLE_GLTF_REGRESSION to verify a local legacy model"]
    fn imports_external_legacy_model() {
        let path = std::env::var("TANGIBLE_GLTF_REGRESSION").unwrap();
        let mesh = crate::model::load_model(path).unwrap();
        assert!(!mesh.vertices.is_empty());
        assert!(!mesh.textures.is_empty());
        eprintln!(
            "Imported {} triangles, {} textures; bounds {:?} to {:?}",
            mesh.vertices.len() / 3,
            mesh.textures.len(),
            mesh.bounds_min,
            mesh.bounds_max
        );
    }
}
