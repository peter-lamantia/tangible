//! Adapt supported KHR_animation_pointer channels to the core reader's sampler API.
use super::{legacy, Result};
use gltf::json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
pub(super) enum Property {
    BaseColor,
    Emissive,
    Metallic,
    Roughness,
    AlphaCutoff,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct MaterialTarget {
    pub material: usize,
    pub property: Property,
}
pub(super) type Targets = BTreeMap<(usize, usize), MaterialTarget>;

pub(super) fn parse(bytes: &[u8]) -> Result<(gltf::Gltf, Targets)> {
    let (json, blob) = if bytes.starts_with(b"glTF\x02\0\0\0") {
        let glb = gltf::binary::Glb::from_slice(bytes)?;
        (glb.json, glb.bin)
    } else if bytes.starts_with(b"glTF") {
        return Ok((legacy::parse(bytes)?, Targets::new()));
    } else {
        (std::borrow::Cow::Borrowed(bytes), None)
    };
    let mut root: Value = gltf::json::deserialize::from_slice(&json)?;
    let has_pointers = root["animations"].as_array().is_some_and(|animations| {
        animations.iter().any(|a| {
            a["channels"]
                .as_array()
                .is_some_and(|channels| channels.iter().any(|c| c["target"]["path"] == "pointer"))
        })
    });
    if !has_pointers {
        drop(root);
        return Ok((legacy::parse(bytes)?, Targets::new()));
    }
    let node_count = root["nodes"].as_array().map_or(0, Vec::len);
    let material_count = root["materials"].as_array().map_or(0, Vec::len);
    let accessors = root["accessors"]
        .as_array()
        .ok_or("Missing animation accessors")?
        .clone();
    let mut targets = Targets::new();
    // Unreferenced placeholder: material channels use the reader's vec4/vec3/scalar
    // output decoding, but are intercepted before applying any node transforms.
    let dummy = node_count;
    let mut needs_dummy = false;
    for (animation_index, animation) in root["animations"]
        .as_array_mut()
        .ok_or("Invalid glTF animations")?
        .iter_mut()
        .enumerate()
    {
        let samplers = animation["samplers"]
            .as_array()
            .ok_or("Missing animation samplers")?
            .clone();
        for (channel_index, channel) in animation["channels"]
            .as_array_mut()
            .ok_or("Invalid glTF animation channels")?
            .iter_mut()
            .enumerate()
        {
            if channel["target"]["path"] != "pointer" {
                continue;
            }
            let pointer = channel["target"]["extensions"]["KHR_animation_pointer"]["pointer"]
                .as_str()
                .ok_or("Missing KHR_animation_pointer pointer")?;
            let parts: Vec<_> = pointer.split('/').collect();
            let index = parts
                .get(2)
                .and_then(|v| v.parse::<usize>().ok())
                .ok_or_else(|| format!("Invalid animation pointer: {pointer}"))?;
            let (node, path) = match parts.as_slice() {
                ["", "nodes", _, property @ ("translation" | "rotation" | "scale" | "weights")]
                    if index < node_count =>
                {
                    (index, *property)
                }
                ["", "materials", _, rest @ ..] if index < material_count => {
                    let (property, path) = match rest {
                        ["pbrMetallicRoughness", "baseColorFactor"] => {
                            (Property::BaseColor, "rotation")
                        }
                        ["pbrMetallicRoughness", "metallicFactor"] => {
                            (Property::Metallic, "weights")
                        }
                        ["pbrMetallicRoughness", "roughnessFactor"] => {
                            (Property::Roughness, "weights")
                        }
                        ["emissiveFactor"] => (Property::Emissive, "translation"),
                        ["alphaCutoff"] => (Property::AlphaCutoff, "weights"),
                        _ => return Err(format!("Unsupported animation pointer: {pointer}").into()),
                    };
                    targets.insert(
                        (animation_index, channel_index),
                        MaterialTarget {
                            material: index,
                            property,
                        },
                    );
                    needs_dummy = true;
                    (dummy, path)
                }
                _ => return Err(format!("Unsupported animation pointer: {pointer}").into()),
            };
            if channel["target"].get("node").is_some() {
                return Err("KHR_animation_pointer targets must omit node".into());
            }
            let sampler = channel["sampler"]
                .as_u64()
                .and_then(|i| samplers.get(i as usize))
                .ok_or("Invalid pointer animation sampler")?;
            let input = sampler["input"]
                .as_u64()
                .and_then(|i| accessors.get(i as usize))
                .ok_or("Invalid pointer animation input accessor")?;
            if input["type"] != "SCALAR" || input["componentType"] != 5126 {
                return Err("Animation pointer times require a float SCALAR accessor".into());
            }
            let output = sampler["output"]
                .as_u64()
                .and_then(|i| accessors.get(i as usize))
                .ok_or("Invalid pointer animation output accessor")?;
            let expected = match path {
                "rotation" => "VEC4",
                "translation" | "scale" => "VEC3",
                _ => "SCALAR",
            };
            if output["type"] != expected || output["componentType"] != 5126 {
                return Err(format!(
                    "Animation pointer {pointer} requires a float {expected} output accessor"
                )
                .into());
            }
            channel["target"] = Value::Object(
                [("node".into(), node.into()), ("path".into(), path.into())]
                    .into_iter()
                    .collect(),
            );
        }
    }
    if needs_dummy {
        if root.get("nodes").is_none() {
            root["nodes"] = Vec::<Value>::new().into();
        }
        root["nodes"]
            .as_array_mut()
            .ok_or("Invalid glTF nodes")?
            .push(Value::Object(Default::default()));
    }
    // The extension has been resolved above. Other required extensions retain
    // their normal validation, as do all core resources and accessor references.
    for key in ["extensionsRequired", "extensionsUsed"] {
        if let Some(extensions) = root.get_mut(key).and_then(Value::as_array_mut) {
            extensions.retain(|v| v != "KHR_animation_pointer");
        }
    }
    let mut source = gltf::Gltf::from_slice(&gltf::json::serialize::to_vec(&root)?)?;
    source.blob = blob.map(|b| b.into_owned());
    Ok((source, targets))
}

#[cfg(test)]
mod tests {
    use super::*;
    const JSON: &[u8] = include_bytes!("../../../tests/fixtures/gltf-sample-assets/AnimatedColorsCube/glTF/AnimatedColorsCube.gltf");

    #[test]
    fn animated_colors_cube_changes_color_and_keeps_node_motion() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "tests/fixtures/gltf-sample-assets/AnimatedColorsCube/glTF/AnimatedColorsCube.gltf",
        );
        let mesh = crate::model::load_model(path).unwrap();
        assert_eq!(mesh.vertices.len(), 144);
        let scene = mesh.animation.as_ref().unwrap();
        let first = scene.evaluate(0.).unwrap();
        let green = scene.evaluate(0.5).unwrap();
        let blue = scene.evaluate(1.5).unwrap();
        let animated = first.batches.last().unwrap().range.start;
        assert!(first.vertices[animated].color[0] > 0.7);
        assert!(green.vertices[animated].color[1] > 0.7);
        assert!(blue.vertices[animated].color[2] > 0.7);
        // Base colors are vectors, not quaternions: interpolate without normalizing.
        let between = scene.evaluate(0.25).unwrap();
        assert!((between.vertices[animated].color[3] - 1.).abs() < 1e-6);
        assert!((between.vertices[animated].color[0] - 0.41).abs() < 1e-6);
        assert!((between.vertices[animated].color[1] - 0.41).abs() < 1e-6);
        assert_ne!(
            first.vertices[animated].position,
            green.vertices[animated].position
        );
        assert_eq!(first.vertices[0].color, blue.vertices[0].color);
        assert_eq!(
            scene.evaluate(3.5).unwrap().vertices[animated].color,
            green.vertices[animated].color
        );
    }

    #[test]
    fn material_only_scalar_and_emissive_animations_update_render_attributes() {
        let dir =
            std::env::temp_dir().join(format!("tangible-material-pointer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("AnimatedColorsCube.bin"),include_bytes!("../../../tests/fixtures/gltf-sample-assets/AnimatedColorsCube/glTF/AnimatedColorsCube.bin")).unwrap();
        let path = dir.join("material.gltf");
        for (property, kind) in [
            ("pbrMetallicRoughness/metallicFactor", "SCALAR"),
            ("pbrMetallicRoughness/roughnessFactor", "SCALAR"),
            ("alphaCutoff", "SCALAR"),
            ("emissiveFactor", "VEC3"),
        ] {
            let mut root: Value = gltf::json::deserialize::from_slice(JSON).unwrap();
            let mut channel = root["animations"][0]["channels"][2].clone();
            channel["target"]["extensions"]["KHR_animation_pointer"]["pointer"] =
                format!("/materials/0/{property}").into();
            root["animations"][0]["channels"] = vec![channel].into();
            root["accessors"][13]["type"] = kind.into();
            root["bufferViews"][13]["byteStride"] = 16.into();
            root["materials"][0]["alphaMode"] = "MASK".into();
            std::fs::write(&path, gltf::json::serialize::to_vec(&root).unwrap()).unwrap();
            let mesh = crate::model::load_model(&path).unwrap();
            let animated = mesh.batches.last().unwrap().range.start;
            let vertex = mesh.vertices[animated];
            let value = match property {
                "pbrMetallicRoughness/metallicFactor" => vertex.surface[0],
                "pbrMetallicRoughness/roughnessFactor" => vertex.surface[1],
                "alphaCutoff" => vertex.surface[2],
                _ => vertex.emissive[0],
            };
            assert!((value - 0.8).abs() < 1e-6, "{property}: {value}");
            let later = mesh.animation.unwrap().evaluate(0.5).unwrap();
            let vertex = later.vertices[animated];
            let value = match property {
                "pbrMetallicRoughness/metallicFactor" => vertex.surface[0],
                "pbrMetallicRoughness/roughnessFactor" => vertex.surface[1],
                "alphaCutoff" => vertex.surface[2],
                _ => vertex.emissive[0],
            };
            assert!((value - 0.02).abs() < 1e-6, "{property}: {value}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn validates_pointer_targets_and_accessor_types() {
        let json = String::from_utf8(JSON.to_vec()).unwrap();
        assert!(
            parse(json.replace("/materials/0/", "/materials/999/").as_bytes())
                .unwrap_err()
                .to_string()
                .contains("Unsupported animation pointer")
        );
        assert!(
            parse(json.replace("/baseColorFactor", "/unknown").as_bytes())
                .unwrap_err()
                .to_string()
                .contains("Unsupported animation pointer")
        );
        let mut root: Value = gltf::json::deserialize::from_slice(JSON).unwrap();
        root["accessors"][13]["type"] = "SCALAR".into();
        assert!(parse(&gltf::json::serialize::to_vec(&root).unwrap())
            .unwrap_err()
            .to_string()
            .contains("requires a float VEC4"));
    }

    #[test]
    fn supports_material_only_required_extension_and_glb_container() {
        let mut root: Value = gltf::json::deserialize::from_slice(JSON).unwrap();
        let channel = root["animations"][0]["channels"][2].clone();
        root["animations"][0]["channels"] = vec![channel].into();
        root["extensionsRequired"] = vec![Value::from("KHR_animation_pointer")].into();
        let json = gltf::json::serialize::to_vec(&root).unwrap();
        let (source, targets) = parse(&json).unwrap();
        assert_eq!(source.document.animations().len(), 1);
        assert_eq!(targets.len(), 1);
        let glb = gltf::binary::Glb {
            header: gltf::binary::Header {
                magic: *b"glTF",
                version: 2,
                length: 0,
            },
            json: std::borrow::Cow::Borrowed(&json),
            bin: Some(std::borrow::Cow::Borrowed(&[0; 4])),
        };
        let (source, targets) = parse(&glb.to_vec().unwrap()).unwrap();
        assert_eq!(source.blob.unwrap(), [0; 4]);
        assert_eq!(targets.len(), 1);
    }
}
