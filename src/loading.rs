use crate::{model, preview};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
};
use winit::event_loop::EventLoopProxy;

#[derive(Clone, Default)]
pub(crate) struct LoadContext {
    id: u64,
    latest: Arc<AtomicU64>,
}
impl LoadContext {
    pub fn is_current(&self) -> bool {
        self.latest.load(Ordering::SeqCst) == self.id
    }
    pub fn check(&self) -> crate::mesh::MeshResult<()> {
        if self.is_current() {
            Ok(())
        } else {
            Err("Loading cancelled".into())
        }
    }
}
type Pending = (LoadContext, PathBuf, EventLoopProxy<AppEvent>);
#[derive(Clone)]
pub(crate) struct Loader {
    latest: Arc<AtomicU64>,
    worker: Arc<(Mutex<Option<Pending>>, Condvar)>,
}
impl Loader {
    pub fn new() -> Self {
        let worker: Arc<(Mutex<Option<Pending>>, Condvar)> =
            Arc::new((Mutex::new(None), Condvar::new()));
        let thread_worker = Arc::downgrade(&worker);
        std::thread::spawn(move || loop {
            let Some(worker) = thread_worker.upgrade() else {
                break;
            };
            let (lock, ready) = &*worker;
            let mut pending = lock.lock().unwrap_or_else(|e| e.into_inner());
            while pending.is_none() {
                let result = ready
                    .wait_timeout(pending, std::time::Duration::from_millis(250))
                    .unwrap_or_else(|e| e.into_inner());
                pending = result.0;
                if Arc::strong_count(&worker) == 1 {
                    return;
                }
            }
            let (context, path, proxy) = pending.take().unwrap();
            drop(pending);
            load_file(context, path, proxy);
        });
        Self {
            latest: Arc::new(AtomicU64::new(0)),
            worker,
        }
    }
    pub fn is_current(&self, id: u64) -> bool {
        self.latest.load(Ordering::SeqCst) == id
    }
}

#[derive(Debug)]
pub(crate) enum AppEvent {
    RepaintRequested {
        deadline: std::time::Instant,
        pass: u64,
    },
    GpuUploadReady(crate::renderer::PreparedMesh),
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

impl Loader {
    pub(crate) fn spawn_open_dialog(&self, proxy: EventLoopProxy<AppEvent>) {
        let loader = self.clone();
        std::thread::spawn(move || {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter(
                    "3D Models (*.stl, *.obj, *.3mf, *.off, *.ply, *.gltf, *.glb)",
                    &["stl", "obj", "3mf", "off", "ply", "gltf", "glb"],
                )
                .set_title("Open 3D Model")
                .pick_file()
            {
                loader.spawn_file_load(path, proxy);
            }
        });
    }

    pub(crate) fn spawn_file_load(&self, path: PathBuf, proxy: EventLoopProxy<AppEvent>) {
        let id = self.latest.fetch_add(1, Ordering::SeqCst) + 1;
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let _ = proxy.send_event(AppEvent::LoadingStarted { id, name });
        let (lock, ready) = &*self.worker;
        let mut pending = lock.lock().unwrap_or_else(|e| e.into_inner());
        if self.is_current(id) {
            *pending = Some((
                LoadContext {
                    id,
                    latest: self.latest.clone(),
                },
                path,
                proxy,
            ));
            ready.notify_one();
        }
    }
}

fn load_file(context: LoadContext, path: PathBuf, proxy: EventLoopProxy<AppEvent>) {
    let id = context.id;
    match model::load_document_with_context(&path, context.clone()) {
        Ok(document) => {
            if !context.is_current() {
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
                if !context.is_current() {
                    return;
                }
                let Some(image) = preview::plate_preview(mesh, || context.is_current()) else {
                    return;
                };
                if !context.is_current()
                    || proxy
                        .send_event(AppEvent::PreviewReady { id, plate, image })
                        .is_err()
                {
                    return;
                }
            }
        }
        Err(error) => {
            if context.is_current() {
                let _ = proxy.send_event(AppEvent::LoadError {
                    id,
                    path,
                    error: error.to_string(),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn superseded_requests_are_ignored() {
        let latest = Arc::new(AtomicU64::new(1));
        let first = LoadContext {
            id: 1,
            latest: latest.clone(),
        };
        assert!(first.check().is_ok());
        latest.store(2, Ordering::SeqCst);
        assert!(first.check().is_err());
        assert!(LoadContext { id: 2, latest }.check().is_ok());
    }
}
