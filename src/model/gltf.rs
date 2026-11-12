mod legacy;
mod pointer;
use super::Result;
use crate::mesh::Mesh;
use glam::{Mat4, Vec3};
use std::path::Path;

pub(super) fn load(path: &Path) -> Result<Mesh> {
    let (source, pointer_targets) = pointer::parse(&crate::mesh::read_limited(path)?)?;
    let doc = source.document;
    let mut blob = source.blob;
    let mut buffers = Vec::new();
    for buffer in doc.buffers() {
        let data = gltf::buffer::Data::from_source_and_blob(
            buffer.source(),
            path.parent(),
            &mut blob,
        )
        .map_err(|error| match buffer.source() {
            gltf::buffer::Source::Uri(uri) if !uri.starts_with("data:") => {
                let hint = if matches!(&error, gltf::Error::Io(e) if e.kind() == std::io::ErrorKind::NotFound) {
                    " Keep the .gltf file and its companion files together, preserving their relative paths, or open the .glb version instead."
                } else {
                    ""
                };
                format!("Could not load glTF buffer {uri:?} relative to {}: {error}.{hint}", path.parent().unwrap_or(Path::new(".")).display())
            }
            _ => format!("Could not load glTF buffer {}: {error}", buffer.index()),
        })?;
        // Preserve the validation performed by gltf::import_buffers.
        if data.len() < buffer.length() {
            return Err(gltf::Error::BufferLength {
                buffer: buffer.index(),
                expected: buffer.length(),
                actual: data.len(),
            }
            .into());
        }
        buffers.push(data);
    }
    let mut textures = Vec::new();
    for texture in doc.textures() {
        let image =
            gltf::image::Data::from_source(texture.source().source(), path.parent(), &buffers)?;
        use gltf::image::Format;
        let (channels, bytes) = match image.format {
            Format::R8 => (1, 1),
            Format::R8G8 => (2, 1),
            Format::R8G8B8 => (3, 1),
            Format::R8G8B8A8 => (4, 1),
            Format::R16 => (1, 2),
            Format::R16G16 => (2, 2),
            Format::R16G16B16 => (3, 2),
            Format::R16G16B16A16 => (4, 2),
            Format::R32G32B32FLOAT => (3, 4),
            Format::R32G32B32A32FLOAT => (4, 4),
        };
        if image.width > 8192 || image.height > 8192 {
            return Err("glTF texture exceeds 8192 pixels".into());
        }
        let mut pixels = Vec::with_capacity(image.width as usize * image.height as usize * 4);
        for p in image.pixels.chunks_exact(channels * bytes) {
            let component = |i: usize| {
                let p = &p[i * bytes..(i + 1) * bytes];
                match bytes {
                    1 => p[0],
                    2 => (u16::from_ne_bytes(p.try_into().unwrap()) / 257) as u8,
                    _ => (f32::from_ne_bytes(p.try_into().unwrap()).clamp(0., 1.) * 255.).round()
                        as u8,
                }
            };
            let r = component(0);
            // One/two-channel images are luminance/luminance-alpha.
            pixels.extend_from_slice(&[
                r,
                if channels >= 3 { component(1) } else { r },
                if channels >= 3 { component(2) } else { r },
                if channels == 2 {
                    component(1)
                } else if channels == 4 {
                    component(3)
                } else {
                    255
                },
            ]);
        }
        let wrap = |v| match v {
            gltf::texture::WrappingMode::ClampToEdge => 0,
            gltf::texture::WrappingMode::MirroredRepeat => 2,
            _ => 1,
        };
        textures.push(crate::mesh::Texture {
            width: image.width,
            height: image.height,
            pixels,
            linear: [
                texture.sampler().mag_filter() != Some(gltf::texture::MagFilter::Nearest),
                !matches!(
                    texture.sampler().min_filter(),
                    Some(
                        gltf::texture::MinFilter::Nearest
                            | gltf::texture::MinFilter::NearestMipmapNearest
                            | gltf::texture::MinFilter::NearestMipmapLinear
                    )
                ),
            ],
            wrap: [
                wrap(texture.sampler().wrap_s()),
                wrap(texture.sampler().wrap_t()),
            ],
        });
    }
    let scene = std::sync::Arc::new(Scene {
        doc,
        pointer_targets,
        buffers,
        textures: std::sync::Arc::new(textures),
    });
    let mut mesh = scene.evaluate(0.)?;
    if scene.doc.animations().next().is_some() {
        mesh.animation = Some(scene);
    }
    Ok(mesh)
}

#[derive(Debug)]
pub(crate) struct Scene {
    doc: gltf::Document,
    pointer_targets: pointer::Targets,
    buffers: Vec<gltf::buffer::Data>,
    textures: std::sync::Arc<Vec<crate::mesh::Texture>>,
}
impl Scene {
    pub(crate) fn evaluate(&self, seconds: f32) -> Result<Mesh> {
        use gltf::animation::util::ReadOutputs;
        let data = |b: gltf::Buffer<'_>| self.buffers.get(b.index()).map(|b| b.0.as_slice());
        let mut transforms: Vec<_> = self
            .doc
            .nodes()
            .map(|n| n.transform().decomposed())
            .collect();
        let mut morph_weights: Vec<Vec<f32>> = self
            .doc
            .nodes()
            .map(|n| {
                n.weights()
                    .map(|v| v.to_vec())
                    .or_else(|| n.mesh().and_then(|m| m.weights().map(|v| v.to_vec())))
                    .unwrap_or_default()
            })
            .collect();
        let mut materials: Vec<_> = self
            .doc
            .materials()
            .map(|m| {
                let pbr = m.pbr_metallic_roughness();
                (
                    pbr.base_color_factor(),
                    m.emissive_factor(),
                    pbr.metallic_factor(),
                    pbr.roughness_factor(),
                    m.alpha_cutoff().unwrap_or(0.5),
                )
            })
            .collect();
        if let Some(animation) = self.doc.animations().next() {
            let duration = animation
                .channels()
                .filter_map(|c| c.reader(data).read_inputs()?.last())
                .fold(0f32, f32::max);
            let t = if duration > 0. {
                seconds % duration
            } else {
                0.
            };
            for (channel_index, channel) in animation.channels().enumerate() {
                let reader = channel.reader(data);
                let times: Vec<_> = reader
                    .read_inputs()
                    .ok_or("Missing animation times")?
                    .collect();
                if times.is_empty() {
                    continue;
                }
                if !times.iter().all(|v| v.is_finite()) || times.windows(2).any(|w| w[0] >= w[1]) {
                    return Err("Invalid glTF animation times".into());
                }
                let i = times.partition_point(|x| *x <= t).saturating_sub(1);
                let j = (i + 1).min(times.len() - 1);
                let dt = times[j] - times[i];
                let f = if dt > 0. {
                    ((t - times[i]) / dt).clamp(0., 1.)
                } else {
                    0.
                };
                let interpolation = channel.sampler().interpolation();
                let cubic = interpolation == gltf::animation::Interpolation::CubicSpline;
                let sample = |values: Vec<[f32; 4]>, rotation: bool| -> Result<[f32; 4]> {
                    if values.len() != times.len() * if cubic { 3 } else { 1 } {
                        return Err("Invalid glTF animation output count".into());
                    }
                    let index = |k| if cubic { k * 3 + 1 } else { k };
                    let a = glam::Vec4::from_array(values[index(i)]);
                    let b = glam::Vec4::from_array(values[index(j)]);
                    let v = if cubic {
                        let m = glam::Vec4::from_array(values[i * 3 + 2]) * dt;
                        let n = glam::Vec4::from_array(values[j * 3]) * dt;
                        a * (2. * f * f * f - 3. * f * f + 1.)
                            + m * (f * f * f - 2. * f * f + f)
                            + b * (-2. * f * f * f + 3. * f * f)
                            + n * (f * f * f - f * f)
                    } else if interpolation == gltf::animation::Interpolation::Step {
                        a
                    } else if rotation {
                        return Ok(glam::Quat::from_array(a.to_array())
                            .slerp(glam::Quat::from_array(b.to_array()), f)
                            .to_array());
                    } else {
                        a.lerp(b, f)
                    };
                    Ok(if rotation {
                        v.normalize().to_array()
                    } else {
                        v.to_array()
                    })
                };
                if let Some(target) = self
                    .pointer_targets
                    .get(&(animation.index(), channel_index))
                {
                    let values = match reader
                        .read_outputs()
                        .ok_or("Missing pointer animation values")?
                    {
                        ReadOutputs::Rotations(v) => v.into_f32().collect(),
                        ReadOutputs::Translations(v) => v.map(|v| [v[0], v[1], v[2], 0.]).collect(),
                        ReadOutputs::MorphTargetWeights(v) => {
                            v.into_f32().map(|v| [v, 0., 0., 0.]).collect()
                        }
                        _ => return Err("Invalid pointer animation output type".into()),
                    };
                    let value = sample(values, false)?;
                    if !value.iter().all(|v| v.is_finite()) {
                        return Err("Non-finite animated material value".into());
                    }
                    let m = materials
                        .get_mut(target.material)
                        .ok_or("Invalid animated material index")?;
                    match target.property {
                        pointer::Property::BaseColor => m.0 = value,
                        pointer::Property::Emissive => m.1 = value[..3].try_into().unwrap(),
                        pointer::Property::Metallic => m.2 = value[0],
                        pointer::Property::Roughness => m.3 = value[0],
                        pointer::Property::AlphaCutoff => m.4 = value[0],
                    }
                    continue;
                }
                let transform = &mut transforms[channel.target().node().index()];
                match reader.read_outputs().ok_or("Missing animation values")? {
                    ReadOutputs::Translations(v) => {
                        transform.0 = sample(v.map(|v| [v[0], v[1], v[2], 0.]).collect(), false)?
                            [..3]
                            .try_into()
                            .unwrap()
                    }
                    ReadOutputs::Scales(v) => {
                        transform.2 = sample(v.map(|v| [v[0], v[1], v[2], 0.]).collect(), false)?
                            [..3]
                            .try_into()
                            .unwrap()
                    }
                    ReadOutputs::Rotations(v) => {
                        transform.1 = sample(v.into_f32().collect(), true)?
                    }
                    ReadOutputs::MorphTargetWeights(v) => {
                        let values: Vec<_> = v.into_f32().collect();
                        let count = channel
                            .target()
                            .node()
                            .mesh()
                            .and_then(|m| m.primitives().next())
                            .map_or(0, |p| p.morph_targets().len());
                        if values.len() != count * times.len() * if cubic { 3 } else { 1 } {
                            return Err("Invalid glTF morph animation output count".into());
                        }
                        let mut weights = Vec::with_capacity(count);
                        for target in 0..count {
                            weights.push(
                                sample(
                                    values
                                        .iter()
                                        .skip(target)
                                        .step_by(count)
                                        .map(|v| [*v, 0., 0., 0.])
                                        .collect(),
                                    false,
                                )?[0],
                            );
                        }
                        morph_weights[channel.target().node().index()] = weights;
                    }
                }
            }
        }
        let scene = self
            .doc
            .default_scene()
            .or_else(|| self.doc.scenes().next())
            .ok_or("glTF has no scene")?;
        let mut worlds = vec![Mat4::IDENTITY; transforms.len()];
        let mut visible = Vec::new();
        let mut stack: Vec<_> = scene.nodes().map(|n| (n, Mat4::IDENTITY, 0usize)).collect();
        while let Some((node, parent, depth)) = stack.pop() {
            crate::mesh::check_cancelled()?;
            if depth > 256 {
                return Err("glTF node hierarchy too deep".into());
            }
            let (t, r, s) = transforms[node.index()];
            let matrix = if self.doc.animations().next().is_some() {
                Mat4::from_scale_rotation_translation(s.into(), glam::Quat::from_array(r), t.into())
            } else {
                Mat4::from_cols_array_2d(&node.transform().matrix())
            };
            let world = parent * matrix;
            worlds[node.index()] = world;
            stack.extend(node.children().map(|n| (n, world, depth + 1)));
            visible.push(node);
        }
        let mut output = Mesh::triangle_builder(0);
        output.textures = self.textures.clone();
        for node in visible {
            let world = worlds[node.index()];
            let joints = node.skin().map(|skin| {
                let inverse: Vec<_> = skin
                    .reader(data)
                    .read_inverse_bind_matrices()
                    .map(|v| v.map(|v| Mat4::from_cols_array_2d(&v)).collect())
                    .unwrap_or_else(|| vec![Mat4::IDENTITY; skin.joints().len()]);
                skin.joints()
                    .zip(inverse)
                    .map(|(n, b)| worlds[n.index()] * b)
                    .collect::<Vec<_>>()
            });
            if let Some(mesh) = node.mesh() {
                for primitive in mesh.primitives() {
                    let reader = primitive.reader(data);
                    let positions: Vec<_> = reader
                        .read_positions()
                        .ok_or("glTF primitive has no positions")?
                        .collect();
                    let normals: Vec<_> = reader
                        .read_normals()
                        .map(|v| v.collect())
                        .unwrap_or_default();
                    let colors: Vec<_> = reader
                        .read_colors(0)
                        .map(|v| v.into_rgba_f32().collect())
                        .unwrap_or_default();
                    let material = primitive.material();
                    let pbr = material.pbr_metallic_roughness();
                    let (base_color, emissive, metallic, roughness, alpha_cutoff) =
                        material.index().map(|i| materials[i]).unwrap_or((
                            pbr.base_color_factor(),
                            material.emissive_factor(),
                            pbr.metallic_factor(),
                            pbr.roughness_factor(),
                            material.alpha_cutoff().unwrap_or(0.5),
                        ));
                    let texture = pbr.base_color_texture();
                    let mr_texture = pbr.metallic_roughness_texture();
                    let normal_texture = material.normal_texture();
                    let occlusion_texture = material.occlusion_texture();
                    let emissive_texture = material.emissive_texture();
                    let read_uv = |set: u32,
                                   transform: Option<gltf::texture::TextureTransform<'_>>|
                     -> Vec<[f32; 2]> {
                        let set = transform
                            .as_ref()
                            .and_then(|t| t.tex_coord())
                            .unwrap_or(set);
                        reader
                            .read_tex_coords(set)
                            .map(|uv| {
                                uv.into_f32()
                                    .map(|uv| {
                                        if let Some(t) = &transform {
                                            let p = glam::Vec2::from_array(uv)
                                                * glam::Vec2::from_array(t.scale());
                                            let (s, c) = t.rotation().sin_cos();
                                            (glam::Vec2::new(c * p.x - s * p.y, s * p.x + c * p.y)
                                                + glam::Vec2::from_array(t.offset()))
                                            .to_array()
                                        } else {
                                            uv
                                        }
                                    })
                                    .collect()
                            })
                            .unwrap_or_default()
                    };
                    let uv = texture
                        .as_ref()
                        .map(|t| read_uv(t.tex_coord(), t.texture_transform()))
                        .unwrap_or_default();
                    let map_uv = [
                        mr_texture
                            .as_ref()
                            .map(|t| read_uv(t.tex_coord(), t.texture_transform()))
                            .unwrap_or_default(),
                        normal_texture
                            .as_ref()
                            .map(|t| read_uv(t.tex_coord(), None))
                            .unwrap_or_default(),
                        occlusion_texture
                            .as_ref()
                            .map(|t| read_uv(t.tex_coord(), None))
                            .unwrap_or_default(),
                        emissive_texture
                            .as_ref()
                            .map(|t| read_uv(t.tex_coord(), t.texture_transform()))
                            .unwrap_or_default(),
                    ];
                    let joint_ids: Vec<_> = reader
                        .read_joints(0)
                        .map(|v| v.into_u16().collect())
                        .unwrap_or_default();
                    let weights: Vec<_> = reader
                        .read_weights(0)
                        .map(|v| v.into_f32().collect())
                        .unwrap_or_default();
                    let tangents: Vec<_> = reader
                        .read_tangents()
                        .map(|v| v.collect())
                        .unwrap_or_default();
                    let morphs: Vec<_> = reader
                        .read_morph_targets()
                        .map(|(p, n, t)| {
                            (
                                p.map(|v| v.collect::<Vec<_>>()).unwrap_or_default(),
                                n.map(|v| v.collect::<Vec<_>>()).unwrap_or_default(),
                                t.map(|v| v.collect::<Vec<_>>()).unwrap_or_default(),
                            )
                        })
                        .collect();
                    let mut points = Vec::with_capacity(positions.len());
                    for (i, p) in positions.iter().enumerate() {
                        if i.is_multiple_of(4096) {
                            crate::mesh::check_cancelled()?;
                        }
                        let mut position = Vec3::from_array(*p);
                        let mut normal = normals.get(i).copied().map(Vec3::from_array);
                        let mut tangent = tangents.get(i).map(|t| Vec3::new(t[0], t[1], t[2]));
                        for (target, (positions, normals, tangents)) in morphs.iter().enumerate() {
                            let weight = morph_weights[node.index()]
                                .get(target)
                                .copied()
                                .unwrap_or(0.);
                            if let Some(p) = positions.get(i) {
                                position += Vec3::from_array(*p) * weight;
                            }
                            if let (Some(n), Some(delta)) = (&mut normal, normals.get(i)) {
                                *n += Vec3::from_array(*delta) * weight;
                            }
                            if let (Some(t), Some(delta)) = (&mut tangent, tangents.get(i)) {
                                *t += Vec3::from_array(*delta) * weight;
                            }
                        }
                        let mut matrix = world;
                        if let (Some(joints), Some(ids), Some(weights)) =
                            (&joints, joint_ids.get(i), weights.get(i))
                        {
                            let total: f32 = weights.iter().sum();
                            if total > 0. {
                                matrix = Mat4::ZERO;
                                for (&id, &weight) in ids.iter().zip(weights) {
                                    matrix += *joints
                                        .get(id as usize)
                                        .ok_or("glTF joint index out of range")?
                                        * (weight / total);
                                }
                            }
                        }
                        points.push((
                            matrix.transform_point3(position).to_array(),
                            normal.map(|n| {
                                matrix
                                    .inverse()
                                    .transpose()
                                    .transform_vector3(n)
                                    .normalize_or_zero()
                                    .to_array()
                            }),
                            tangent.map(|t| {
                                let t = matrix.transform_vector3(t).normalize_or_zero();
                                [
                                    t.x,
                                    t.y,
                                    t.z,
                                    tangents[i][3]
                                        * if matrix.determinant() < 0. { -1. } else { 1. },
                                ]
                            }),
                        ));
                    }
                    let indices: Vec<usize> = reader
                        .read_indices()
                        .map(|v| v.into_u32().map(|v| v as usize).collect())
                        .unwrap_or_else(|| (0..points.len()).collect());
                    let mode = primitive.mode();
                    let topology = match mode {
                        gltf::mesh::Mode::Points => 2,
                        gltf::mesh::Mode::Lines
                        | gltf::mesh::Mode::LineLoop
                        | gltf::mesh::Mode::LineStrip => 1,
                        _ => 0,
                    };
                    let start = output.vertices.len();
                    let mut emit = |ids: &[usize]| -> Result<()> {
                        if output.vertices.len().is_multiple_of(3072) {
                            crate::mesh::check_cancelled()?;
                        }
                        if output.vertices.len() + ids.len() > crate::mesh::MAX_TRIANGLES * 3 {
                            return Err("Model exceeds vertex limit".into());
                        }
                        let mut vertices = Vec::new();
                        for &i in ids {
                            let &(position, normal, tangent) =
                                points.get(i).ok_or("glTF index out of range")?;
                            if !position.iter().all(|v| v.is_finite()) {
                                return Err("Non-finite vertex coordinate".into());
                            }
                            let mut color = glam::Vec4::from_array(base_color)
                                * glam::Vec4::from_array(colors.get(i).copied().unwrap_or([1.; 4]));
                            if material.alpha_mode() == gltf::material::AlphaMode::Opaque {
                                color.w = 1.;
                            }
                            vertices.push(crate::mesh::Vertex {
                                position,
                                normal: normal.unwrap_or([0.; 3]),
                                tangent: tangent.unwrap_or([0.; 4]),
                                color: color.to_array(),
                                uv: uv.get(i).copied().unwrap_or([0.; 2]),
                                surface: [
                                    metallic,
                                    roughness,
                                    if material.alpha_mode() == gltf::material::AlphaMode::Mask {
                                        alpha_cutoff
                                    } else if material.alpha_mode()
                                        == gltf::material::AlphaMode::Opaque
                                    {
                                        -2.
                                    } else {
                                        -1.
                                    },
                                    f32::from(material.unlit() || topology != 0),
                                ],
                                emissive,
                                map_uv: std::array::from_fn(|slot| {
                                    map_uv[slot].get(i).copied().unwrap_or([0.; 2])
                                }),
                                map_factors: [
                                    normal_texture.as_ref().map_or(0., |t| t.scale()),
                                    occlusion_texture.as_ref().map_or(1., |t| t.strength()),
                                ],
                            });
                        }
                        if topology == 0 && world.determinant() < 0. {
                            vertices.swap(1, 2);
                        }
                        if topology == 0 && normals.is_empty() {
                            let n = (Vec3::from_array(vertices[1].position)
                                - Vec3::from_array(vertices[0].position))
                            .cross(
                                Vec3::from_array(vertices[2].position)
                                    - Vec3::from_array(vertices[0].position),
                            )
                            .normalize_or_zero()
                            .to_array();
                            for v in &mut vertices {
                                v.normal = n;
                            }
                        }
                        for v in vertices {
                            output.bounds_min = output.bounds_min.min(v.position.into());
                            output.bounds_max = output.bounds_max.max(v.position.into());
                            std::sync::Arc::make_mut(&mut output.vertices).push(v);
                        }
                        Ok(())
                    };
                    use gltf::mesh::Mode;
                    match mode {
                        Mode::Triangles => {
                            if !indices.len().is_multiple_of(3) {
                                return Err("Invalid glTF triangle count".into());
                            }
                            for c in indices.as_chunks::<3>().0 {
                                emit(c)?;
                            }
                        }
                        Mode::TriangleStrip => {
                            for (i, c) in indices.windows(3).enumerate() {
                                if i % 2 == 0 {
                                    emit(c)?;
                                } else {
                                    emit(&[c[1], c[0], c[2]])?;
                                }
                            }
                        }
                        Mode::TriangleFan => {
                            for c in indices.get(1..).unwrap_or_default().windows(2) {
                                emit(&[indices[0], c[0], c[1]])?;
                            }
                        }
                        Mode::Points => {
                            for i in &indices {
                                emit(&[*i])?;
                            }
                        }
                        Mode::Lines => {
                            if !indices.len().is_multiple_of(2) {
                                return Err("Invalid glTF line count".into());
                            }
                            for c in indices.as_chunks::<2>().0 {
                                emit(c)?;
                            }
                        }
                        Mode::LineStrip | Mode::LineLoop => {
                            for c in indices.windows(2) {
                                emit(c)?;
                            }
                            if mode == Mode::LineLoop && indices.len() > 1 {
                                emit(&[*indices.last().unwrap(), indices[0]])?;
                            }
                        }
                    }
                    output.batches.push(crate::mesh::Batch {
                        range: start..output.vertices.len(),
                        topology,
                        texture: texture.map(|t| t.texture().index()),
                        maps: [
                            mr_texture.map(|t| t.texture().index()),
                            normal_texture.map(|t| t.texture().index()),
                            occlusion_texture.map(|t| t.texture().index()),
                            emissive_texture.map(|t| t.texture().index()),
                        ],
                        double_sided: material.double_sided(),
                        blend: material.alpha_mode() == gltf::material::AlphaMode::Blend,
                    });
                }
            }
        }
        output.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_companion_buffer_names_resource_and_recovery() {
        let dir =
            std::env::temp_dir().join(format!("tangible-gltf-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Box.gltf");
        std::fs::write(
            &path,
            include_bytes!("../../tests/fixtures/gltf-sample-assets/Box/glTF/Box.gltf"),
        )
        .unwrap();
        let error = crate::model::load_model(&path).unwrap_err().to_string();
        assert!(error.contains("Box0.bin"), "{error}");
        assert!(error.contains(&dir.display().to_string()), "{error}");
        assert!(error.contains("companion files together"), "{error}");
        assert!(error.contains(".glb"), "{error}");
        // A truncated companion must still fail the buffer length validation.
        std::fs::write(dir.join("Box0.bin"), [0u8; 4]).unwrap();
        let error = load(&path).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<gltf::Error>(),
            Some(gltf::Error::BufferLength { .. })
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn imports_khronos_sample_assets() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gltf-sample-assets");
        let cases = [
            ("Triangle/glTF/Triangle.gltf", 1, [0.; 3], [1., 1., 0.]),
            (
                "Triangle/glTF-Embedded/Triangle.gltf",
                1,
                [0.; 3],
                [1., 1., 0.],
            ),
            (
                "TriangleWithoutIndices/glTF/TriangleWithoutIndices.gltf",
                1,
                [0.; 3],
                [1., 1., 0.],
            ),
            ("Box/glTF/Box.gltf", 12, [-0.5; 3], [0.5; 3]),
            ("Box/glTF-Binary/Box.glb", 12, [-0.5; 3], [0.5; 3]),
            (
                "BoxInterleaved/glTF/BoxInterleaved.gltf",
                12,
                [-0.5; 3],
                [0.5; 3],
            ),
            (
                "SimpleSparseAccessor/glTF/SimpleSparseAccessor.gltf",
                12,
                [0.; 3],
                [6., 4., 0.],
            ),
        ];
        for (file, triangles, min, max) in cases {
            // Exercise the same extension dispatch used when opening a model.
            let mesh = crate::model::load_model(root.join(file))
                .unwrap_or_else(|error| panic!("{file}: {error}"));
            assert_eq!(mesh.vertices.len(), triangles * 3, "{file}");
            assert!(
                mesh.bounds_min.abs_diff_eq(Vec3::from_array(min), 1e-6),
                "{file}: minimum bounds"
            );
            assert!(
                mesh.bounds_max.abs_diff_eq(Vec3::from_array(max), 1e-6),
                "{file}: maximum bounds"
            );
            for vertex in mesh.vertices.iter() {
                assert!(Vec3::from_array(vertex.position).is_finite(), "{file}");
                assert!(
                    (Vec3::from_array(vertex.normal).length() - 1.).abs() < 1e-6,
                    "{file}: unit normal"
                );
            }
        }
    }

    #[test]
    fn imports_transformed_external_buffer() {
        let dir = std::env::temp_dir().join(format!("tangible-gltf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut bytes = Vec::new();
        for value in [0f32, 0., 0., 1., 0., 0., 0., 1., 0.] {
            bytes.extend(value.to_le_bytes());
        }
        std::fs::write(dir.join("mesh.bin"), bytes).unwrap();
        std::fs::write(dir.join("mesh.gltf"), r#"{"asset":{"version":"2.0"},"buffers":[{"uri":"mesh.bin","byteLength":36}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"images":[{"uri":"unused-texture.png"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],"nodes":[{"mesh":0,"translation":[2,0,0]}],"scenes":[{"nodes":[0]}],"scene":0}"#).unwrap();
        let mesh = load(&dir.join("mesh.gltf")).unwrap();
        assert_eq!(mesh.bounds_min.x, 2.0);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod rendering_tests {
    use super::*;

    fn animated_asset(mode: u32) -> (std::path::PathBuf, Mesh) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "tangible-scene-{}-{mode}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut bytes = Vec::new();
        for p in [0f32, 0., 0., 1., 0., 0., 0., 1., 0.] {
            bytes.extend(p.to_le_bytes());
        }
        bytes.extend([0u8; 24]); // JOINTS_0: three u16 vec4s
        for w in [1f32, 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.] {
            bytes.extend(w.to_le_bytes());
        }
        for t in [0f32, 2., 0., 0., 0., 2., 0., 0.] {
            bytes.extend(t.to_le_bytes());
        }
        std::fs::write(dir.join("scene.bin"), bytes).unwrap();
        let json = r#"{
          "asset":{"version":"2.0"},
          "buffers":[{"uri":"scene.bin","byteLength":140}],
          "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":24},{"buffer":0,"byteOffset":60,"byteLength":48},{"buffer":0,"byteOffset":108,"byteLength":8},{"buffer":0,"byteOffset":116,"byteLength":24}],
          "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},{"bufferView":1,"componentType":5123,"count":3,"type":"VEC4"},{"bufferView":2,"componentType":5126,"count":3,"type":"VEC4"},{"bufferView":3,"componentType":5126,"count":2,"type":"SCALAR","min":[0],"max":[2]},{"bufferView":4,"componentType":5126,"count":2,"type":"VEC3"}],
          "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[1,0.2,0.1,1],"metallicFactor":0.3,"roughnessFactor":0.7},"emissiveFactor":[0.1,0,0]}],
          "meshes":[{"primitives":[{"mode":MODE,"attributes":{"POSITION":0,"JOINTS_0":1,"WEIGHTS_0":2},"material":0}]}],
          "skins":[{"joints":[1]}],"nodes":[{"mesh":0,"skin":0},{}],"scenes":[{"nodes":[0,1]}],"scene":0,
          "animations":[{"samplers":[{"input":3,"output":4,"interpolation":"LINEAR"}],"channels":[{"sampler":0,"target":{"node":1,"path":"translation"}}]}]
        }"#.replace("MODE", &mode.to_string());
        std::fs::write(dir.join("scene.gltf"), json).unwrap();
        let mesh = load(&dir.join("scene.gltf")).unwrap();
        (dir, mesh)
    }

    #[test]
    fn supplied_tangents_survive_skinning_and_mirrored_transforms() {
        let (dir, _) = animated_asset(4);
        let path = dir.join("scene.gltf");
        let mut root: gltf::json::Value =
            gltf::json::deserialize::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let mut bytes = std::fs::read(dir.join("scene.bin")).unwrap();
        let offset = bytes.len();
        for _ in 0..3 {
            for value in [1f32, 0., 0., -1.] {
                bytes.extend(value.to_le_bytes());
            }
        }
        root["buffers"][0]["byteLength"] = bytes.len().into();
        let view = root["bufferViews"].as_array().unwrap().len();
        root["bufferViews"].as_array_mut().unwrap().push(
            gltf::json::deserialize::from_str(&format!(
                r#"{{"buffer":0,"byteOffset":{offset},"byteLength":48}}"#
            ))
            .unwrap(),
        );
        let accessor = root["accessors"].as_array().unwrap().len();
        root["accessors"].as_array_mut().unwrap().push(
            gltf::json::deserialize::from_str(&format!(
                r#"{{"bufferView":{view},"componentType":5126,"count":3,"type":"VEC4"}}"#
            ))
            .unwrap(),
        );
        root["meshes"][0]["primitives"][0]["attributes"]["TANGENT"] = accessor.into();
        std::fs::write(dir.join("scene.bin"), bytes).unwrap();
        std::fs::write(&path, gltf::json::serialize::to_vec(&root).unwrap()).unwrap();
        let mesh = load(&path).unwrap();
        assert_eq!(mesh.vertices[0].tangent, [1., 0., 0., -1.]);
        assert_eq!(
            mesh.animation.unwrap().evaluate(1.).unwrap().vertices[0].tangent,
            [1., 0., 0., -1.]
        );
        root.as_object_mut().unwrap().remove("animations");
        root["nodes"][0].as_object_mut().unwrap().remove("skin");
        root["nodes"][0]["scale"] = vec![-2., 3., 1.].into();
        std::fs::write(&path, gltf::json::serialize::to_vec(&root).unwrap()).unwrap();
        let reflected = load(&path).unwrap();
        assert_eq!(reflected.vertices[0].tangent, [-1., 0., 0., 1.]);
        assert_eq!(reflected.vertices[0].normal, [0., 0., 1.]);
        let fallback = Mesh::from_triangles([[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]]).unwrap();
        assert_eq!(fallback.vertices[0].tangent, [0.; 4]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn textures_and_sampler_survive_import() {
        let (dir, _) = animated_asset(4);
        std::fs::write(
            dir.join("color.png"),
            include_bytes!("../../assets/tangible-ui.png"),
        )
        .unwrap();
        let path = dir.join("scene.gltf");
        let json = std::fs::read_to_string(&path).unwrap()
            .replace("\"materials\":", "\"images\":[{\"uri\":\"color.png\"}],\"samplers\":[{\"wrapS\":33071,\"wrapT\":33648}],\"textures\":[{\"source\":0,\"sampler\":0}],\"materials\":")
            .replace("\"baseColorFactor\":", "\"baseColorTexture\":{\"index\":0},\"baseColorFactor\":");
        std::fs::write(&path, json).unwrap();
        let mesh = load(&path).unwrap();
        assert_eq!(mesh.batches[0].texture, Some(0));
        assert_eq!(mesh.textures[0].wrap, [0, 2]);
        assert!(mesh.textures[0].width > 1);
        assert_eq!(
            mesh.textures[0].pixels.len(),
            (mesh.textures[0].width * mesh.textures[0].height * 4) as usize
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn morph_targets_are_applied_before_skinning() {
        let (dir, _) = animated_asset(4);
        let path = dir.join("scene.gltf");
        let mut bytes = std::fs::read(dir.join("scene.bin")).unwrap();
        for value in [0f32, 0., 1., 0., 0., 1., 0., 0., 1.] {
            bytes.extend(value.to_le_bytes());
        }
        std::fs::write(dir.join("scene.bin"), bytes).unwrap();
        let json = std::fs::read_to_string(&path).unwrap()
            .replace("\"byteLength\":140", "\"byteLength\":176")
            .replace("\"bufferViews\":[", "\"bufferViews\":[{\"buffer\":0,\"byteOffset\":140,\"byteLength\":36},")
            .replace("\"bufferView\":4", "\"bufferView\":5").replace("\"bufferView\":3", "\"bufferView\":4").replace("\"bufferView\":2", "\"bufferView\":3").replace("\"bufferView\":1", "\"bufferView\":2").replace("\"bufferView\":0", "\"bufferView\":1")
            .replace("\"accessors\":[", "\"accessors\":[{\"bufferView\":0,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\",\"min\":[0,0,1],\"max\":[0,0,1]},")
            .replace("\"POSITION\":0,\"JOINTS_0\":1,\"WEIGHTS_0\":2", "\"POSITION\":1,\"JOINTS_0\":2,\"WEIGHTS_0\":3")
            .replace("\"input\":3,\"output\":4", "\"input\":4,\"output\":5")
            .replace("\"material\":0", "\"material\":0,\"targets\":[{\"POSITION\":0}]")
            .replace("\"meshes\":[{", "\"meshes\":[{\"weights\":[0.5],");
        std::fs::write(&path, json).unwrap();
        let mesh = load(&path).unwrap();
        assert_eq!(mesh.vertices[0].position, [0., 0., 0.5]);
        assert_eq!(
            mesh.animation.unwrap().evaluate(1.).unwrap().vertices[0].position,
            [1., 0., 0.5]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn step_and_cubic_spline_interpolation() {
        let (dir, _) = animated_asset(4);
        let path = dir.join("scene.gltf");
        let json = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, json.replace("LINEAR", "STEP")).unwrap();
        assert_eq!(
            load(&path)
                .unwrap()
                .animation
                .unwrap()
                .evaluate(1.)
                .unwrap()
                .vertices[0]
                .position,
            [0.; 3]
        );
        let mut bytes = std::fs::read(dir.join("scene.bin")).unwrap();
        bytes.truncate(116);
        for v in [
            0f32, 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 2., 0., 0., 0., 0., 0.,
        ] {
            bytes.extend(v.to_le_bytes());
        }
        std::fs::write(dir.join("scene.bin"), bytes).unwrap();
        let json = json
            .replace("LINEAR", "CUBICSPLINE")
            .replace("\"byteLength\":140", "\"byteLength\":188")
            .replace(
                "\"byteOffset\":116,\"byteLength\":24",
                "\"byteOffset\":116,\"byteLength\":72",
            )
            .replace(
                "\"bufferView\":4,\"componentType\":5126,\"count\":2",
                "\"bufferView\":4,\"componentType\":5126,\"count\":6",
            );
        std::fs::write(&path, json).unwrap();
        assert_eq!(
            load(&path)
                .unwrap()
                .animation
                .unwrap()
                .evaluate(0.5)
                .unwrap()
                .vertices[0]
                .position,
            [0.3125, 0., 0.]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn material_and_joint_animation_are_preserved_and_loop() {
        let (dir, mesh) = animated_asset(4);
        assert_eq!(mesh.vertices[0].color, [1., 0.2, 0.1, 1.]);
        assert_eq!(&mesh.vertices[0].surface[..2], &[0.3, 0.7]);
        assert_eq!(mesh.vertices[0].emissive, [0.1, 0., 0.]);
        let scene = mesh.animation.as_ref().unwrap();
        let animated = scene.evaluate(1.).unwrap();
        assert_eq!(animated.vertices[0].position, [1., 0., 0.]);
        assert_eq!(animated.vertices[1].position, [2., 0., 0.]);
        assert_eq!(
            scene.evaluate(3.).unwrap().vertices[0].position,
            animated.vertices[0].position
        );
        assert_eq!(mesh.vertices[0].position, [0., 0., 0.]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn point_line_strip_and_loop_modes_expand_without_triangles() {
        for (mode, topology, count) in [(0, 2, 3), (2, 1, 6), (3, 1, 4), (5, 0, 3), (6, 0, 3)] {
            let (dir, mesh) = animated_asset(mode);
            assert_eq!(mesh.vertices.len(), count);
            assert_eq!(mesh.batches[0].topology, topology);
            assert_eq!(mesh.batches[0].range, 0..count);
            assert_eq!(
                mesh.animation.unwrap().evaluate(1.).unwrap().vertices[0].position,
                [1., 0., 0.]
            );
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[cfg(test)]
mod material_regressions {
    use super::*;
    fn fixture(name: &str) -> Mesh {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gltf-sample-assets");
        crate::model::load_model(root.join(name).join("glTF").join(format!("{name}.gltf"))).unwrap()
    }
    #[test]
    fn unlit_materials_and_texture_uv_sets_are_preserved() {
        let unlit = fixture("UnlitTest");
        assert!(unlit.vertices.iter().all(|v| v.surface[3] == 1.));
        assert!(unlit.batches.iter().all(|b| !b.double_sided));
        let multi = fixture("MultiUVTest");
        assert_eq!(multi.batches[0].texture, Some(0));
        assert_eq!(multi.batches[0].maps[3], Some(1));
        assert!(multi.vertices.iter().any(|v| v.uv != v.map_uv[3]));
    }
    #[test]
    fn texture_transform_applies_offset_scale_and_rotation() {
        let mesh = fixture("TextureTransformTest");
        // Scene nodes are traversed in reverse order; the last quad is Offset U.
        let offset = mesh.batches.last().unwrap();
        assert!(mesh.vertices[offset.range.clone()]
            .iter()
            .all(|v| v.uv[0] >= 0.5 && v.uv[0] <= 1.));
        // The 'All' quad is visited first. All four raw corners must undergo
        // scale, then rotation, then translation, independently of index order.
        let (s, c) = 0.3f32.sin_cos();
        let expected = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
            .map(|[x, y]| [-0.2 + 1.5 * (c * x - s * y), -0.1 + 1.5 * (s * x + c * y)]);
        for vertex in &mesh.vertices[mesh.batches[0].range.clone()] {
            assert!(
                expected
                    .iter()
                    .any(|uv| (uv[0] - vertex.uv[0]).abs() < 1e-6
                        && (uv[1] - vertex.uv[1]).abs() < 1e-6),
                "unexpected UV {:?}",
                vertex.uv
            );
        }
    }
}
