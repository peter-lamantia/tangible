use crate::{model, preview};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use winit::event_loop::EventLoopProxy;

static LATEST_REQUEST: AtomicU64 = AtomicU64::new(0);

pub(crate) fn is_current(id: u64) -> bool {
    LATEST_REQUEST.load(Ordering::SeqCst) == id
}

#[derive(Debug)]
pub(crate) enum AppEvent {
    LoadingStarted {
        id: u64,
        name: String,
    },
    MeshLoaded {
        id: u64,
        path: PathBuf,
        document: Arc<model::ModelDocument>,
    },
    PreviewReady {
        id: u64,
        plate: Option<usize>,
        image: egui::ColorImage,
    },
    LoadError {
        id: u64,
        path: PathBuf,
        error: String,
    },
}

pub(crate) fn spawn_open_dialog(proxy: EventLoopProxy<AppEvent>) {
    std::thread::spawn(move || {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("3D Models (*.stl, *.obj, *.3mf)", &["stl", "obj", "3mf"])
            .set_title("Open 3D Model")
            .pick_file()
        {
            spawn_file_load(path, proxy);
        }
    });
}

pub(crate) fn spawn_file_load(path: PathBuf, proxy: EventLoopProxy<AppEvent>) {
    let id = LATEST_REQUEST.fetch_add(1, Ordering::SeqCst) + 1;
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let _ = proxy.send_event(AppEvent::LoadingStarted { id, name });
    std::thread::spawn(move || {
        match model::load_document(&path) {
            Ok(document) => {
                if !is_current(id) {
                    return;
                }
                let document = Arc::new(document);
                if proxy
                    .send_event(AppEvent::MeshLoaded {
                        id,
                        path,
                        document: document.clone(),
                    })
                    .is_err()
                {
                    return;
                }
                if document.plates.is_empty() {
                    return;
                }
                // Send each image separately so the model and individual plates appear progressively.
                for (plate, mesh) in std::iter::once((None, &document.mesh)).chain(
                    document
                        .plates
                        .iter()
                        .enumerate()
                        .map(|(i, p)| (Some(i), &p.mesh)),
                ) {
                    if !is_current(id) {
                        return;
                    }
                    let Some(image) = preview::plate_preview(mesh, || is_current(id)) else {
                        return;
                    };
                    if !is_current(id)
                        || proxy
                            .send_event(AppEvent::PreviewReady { id, plate, image })
                            .is_err()
                    {
                        return;
                    }
                }
            }
            Err(error) => {
                if is_current(id) {
                    let _ = proxy.send_event(AppEvent::LoadError {
                        id,
                        path,
                        error: error.to_string(),
                    });
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn superseded_requests_are_ignored() {
        let first = LATEST_REQUEST.fetch_add(1, Ordering::SeqCst) + 1;
        assert!(is_current(first));
        let second = LATEST_REQUEST.fetch_add(1, Ordering::SeqCst) + 1;
        assert!(!is_current(first));
        assert!(is_current(second));
    }
}
