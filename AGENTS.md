# Agent guide

Tangible is a Rust desktop viewer for STL, OBJ, 3MF, OFF, PLY, glTF, and GLB models. Keep changes focused and preserve existing user changes.

## Project map

- `src/main.rs`: window lifecycle, explicit viewer state, input, and UI composition.
- `src/renderer.rs`: GPU resources, material packing, geometry caching, and rendering.
- `src/model.rs`: document loading and explicit loading-context entry point.
- `src/model/`: format loaders, glTF animation, legacy conversion, and resource budgets.
- `src/mesh.rs`: shared geometry, typed materials/topologies, validation, and scoped cancellation bridge.
- `src/loading.rs`: viewer-owned worker, request contexts, and progressive preview delivery.
- `src/preview.rs`: cancellable CPU plate previews.
- `src/stl.rs`: STL parsing; `src/camera.rs`: camera behavior.
- `src/repaint.rs`: deadline-based redraw scheduling for idle efficiency.
- `src/preferences.rs`: platform-specific preference storage.
- `src/ui.rs` and `src/theme.rs`: UI helpers and styling; `src/shader.wgsl`: shader.
- `src/renderer/sample_tests.rs`: optional offscreen sample-model GPU checks.
- `assets/`: bundled artwork and fonts. See `assets/ARTWORK.md` for attribution.

Keep material factors in draw batches rather than duplicating them per vertex.
Convert typed alpha modes to shader encodings only in the renderer. Animated meshes
use reusable, non-indexed GPU buffers; static meshes use bounded deduplication.
Keep geometry deduplication and GPU buffer/texture preparation off the UI thread.
Upload completions must match the current revision before replacing the scene.
Honor egui repaint deadlines; delayed requests must not cause continuous redraws.
The loading context scope must restore the previous context on errors and unwinds.

## Development and checks

Use stable Rust. Always run `cargo clippy --locked --all-targets -- -D warnings` before finishing a task and fix any reported warnings. For changes that affect code, also run `cargo build --locked --all-targets` and `cargo test --locked --all-targets`. Tests live alongside the implementation. On Linux, CI installs `libwayland-dev`, `libxkbcommon-dev`, and `libgtk-3-dev`; a working graphics environment is needed to run the GUI.

CI runs on Linux, macOS, and Windows. Keep platform-specific changes portable. The release workflow builds archives on pushes to `main` and publishes them under commit-specific tags. Update the README when user-facing behavior or distribution changes.
