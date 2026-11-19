use super::Result;
use std::path::Path;

/// A document-wide budget, including companion buffers and decoded images.
pub(super) struct Resources {
    remaining: u64,
}
impl Resources {
    pub fn new() -> Self {
        Self {
            remaining: crate::mesh::MAX_INPUT_BYTES,
        }
    }
    fn charge(&mut self, bytes: u64) -> Result<()> {
        self.remaining = self
            .remaining
            .checked_sub(bytes)
            .ok_or("glTF resources exceed 512 MiB budget")?;
        Ok(())
    }
    pub fn uri(&mut self, uri: &str, base: &Path) -> Result<Vec<u8>> {
        crate::mesh::check_cancelled()?;
        let data = if let Some(data) = uri.strip_prefix("data:") {
            let (_, encoded) = data.split_once(";base64,").ok_or("Unsupported data URI")?;
            if encoded.len() as u64 / 4 * 3 > self.remaining + 2 {
                return Err("glTF resources exceed 512 MiB budget".into());
            }
            base64::decode(encoded)?
        } else {
            let path = if let Some(path) = uri
                .strip_prefix("file://")
                .or_else(|| uri.strip_prefix("file:"))
            {
                std::path::PathBuf::from(path)
            } else {
                if uri.contains(':') {
                    return Err("Unsupported resource URI scheme".into());
                }
                base.join(urlencoding::decode(uri)?.as_ref())
            };
            if std::fs::metadata(&path).map_err(|e| format!("Could not load glTF resource {uri:?} relative to {}: {e}. Keep companion files together or open the .glb version.", base.display()))?.len() > self.remaining {
                return Err("glTF resources exceed 512 MiB budget".into());
            }
            crate::mesh::read_limited(&path).map_err(|e| format!("Could not load glTF resource {uri:?} relative to {}: {e}. Keep companion files in their original relative locations.", base.display()))?
        };
        self.charge(data.len() as u64)?;
        Ok(data)
    }
    pub fn blob(&mut self, data: Vec<u8>) -> Result<Vec<u8>> {
        self.charge(data.len() as u64)?;
        Ok(data)
    }
    pub fn image(
        &mut self,
        source: gltf::image::Source<'_>,
        base: &Path,
        buffers: &[gltf::buffer::Data],
    ) -> Result<gltf::image::Data> {
        crate::mesh::check_cancelled()?;
        let owned;
        let encoded = match source {
            gltf::image::Source::Uri { uri, .. } => {
                owned = self.uri(uri, base)?;
                owned.as_slice()
            }
            gltf::image::Source::View { view, .. } => buffers
                .get(view.buffer().index())
                .and_then(|b| {
                    b.0.get(view.offset()..view.offset().checked_add(view.length())?)
                })
                .ok_or("Invalid image buffer view")?,
        };
        let mut reader =
            image::ImageReader::new(std::io::Cursor::new(encoded)).with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(self.remaining);
        reader.limits(limits);
        let decoded = reader.decode()?;
        self.charge(u64::from(decoded.width()) * u64::from(decoded.height()) * 4)?;
        let rgba = decoded.to_rgba8();
        Ok(gltf::image::Data {
            width: rgba.width(),
            height: rgba.height(),
            format: gltf::image::Format::R8G8B8A8,
            pixels: rgba.into_raw(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resources_share_a_budget() {
        let mut resources = Resources { remaining: 5 };
        assert_eq!(
            resources
                .uri("data:application/octet-stream;base64,AQID", Path::new("."))
                .unwrap(),
            [1, 2, 3]
        );
        assert!(resources.blob(vec![0; 3]).is_err());
    }
    #[test]
    fn malformed_image_views_return_errors() {
        let doc = gltf::Gltf::from_slice(br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":8}],"bufferViews":[{"buffer":0,"byteLength":8}],"images":[{"bufferView":0,"mimeType":"image/png"}]}"#).unwrap();
        assert!(Resources::new()
            .image(doc.images().next().unwrap().source(), Path::new("."), &[])
            .is_err());
    }
}
