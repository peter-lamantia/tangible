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

## Loading and rendering constraints

Loaders build render geometry directly to limit temporary memory; PLY streams
records from disk. Models support up to 10 million expanded triangles. Mesh
copies share immutable vertex storage. Large uploads run in chunks on a background
worker, checking cancellation between chunks or textures. Keep the current model
visible until replacement resources are ready and the loading indicator visible
through GPU preparation.

Static geometry retains source accessor indices where available and uses bounded
deduplication otherwise, sampling large unindexed chunks first. Keep indexed
geometry only when it reduces upload size. The renderer caches up to two recent
meshes of at most 32 MiB each. Plate previews use incremental rasterization with
four-sample antialiasing. Release builds enable thin LTO.

glTF companion buffers, encoded images, and decoded RGBA images share a 512 MiB
budget. Texture dimensions are limited to 8192 pixels; decoding must respect the
remaining allocation budget. Check cancellation between resource imports.
CPU animation currently allocates evaluated geometry each frame, then updates
reusable GPU buffers.

glTF rendering uses approximate direct lighting, core material maps,
`KHR_materials_unlit`, and `KHR_texture_transform` on base-color,
metallic/roughness, and emissive maps. Cull back faces for single-sided materials.
Render transparent chunks after opaque surfaces, sorted back to front with depth
writes disabled; intersecting triangles within a chunk remain approximate.
Preserve authored tangents and handedness through skinning, morphing, and mirrored
transforms; derive a basis from UVs when tangents are absent. Other material
extensions, environment lighting, mipmaps, and animation selection are unsupported.

Legacy glTF/GLB 1.0 supports static models, named resources, node transforms, and
embedded textures. Diffuse colors/textures and shininess are approximated; custom
GLSL techniques are not executed. Legacy animation and skinning require conversion
to glTF 2.0. `KHR_animation_pointer` supports node transforms/morph weights and
material base color, emissive, metallic, roughness, and alpha-cutoff factors with
floating-point output accessors; other targets report specific unsupported-target
errors.

Keep glTF companion resources in their original relative locations. Missing-buffer
errors identify the resource and base directory. Load errors appear in a scrollable
bottom-right popup with copy and dismiss actions.

Preferences are saved on normal window close in `tangible/preferences` beneath
`%APPDATA%` on Windows, `~/Library/Application Support` on macOS, and
`$XDG_CONFIG_HOME` (or `~/.config`) on Linux.

## Development and checks

Use stable Rust. Always run `cargo clippy --locked --all-targets -- -D warnings` before finishing a task and fix any reported warnings. For changes that affect code, also run `cargo build --locked --all-targets` and `cargo test --locked --all-targets`. Tests live alongside the implementation. On Linux, CI installs `libwayland-dev`, `libxkbcommon-dev`, and `libgtk-3-dev`; a working graphics environment is needed to run the GUI.

CI runs on Linux, macOS, and Windows. Keep platform-specific changes portable. The release workflow builds archives on pushes to `main` and publishes them under commit-specific tags. Update the README when user-facing behavior or distribution changes.

### glTF fixtures and optional GPU checks

Offline loader regression tests use a pinned subset of Khronos glTF Sample Assets.
See `tests/fixtures/gltf-sample-assets/README.md` for provenance, licenses, and
coverage. Additional tests cover primitive modes, materials, animation, skinning,
and GPU pipeline validation.

Run the optional offscreen checks against a local Khronos checkout:

```bash
TANGIBLE_GLTF_SAMPLES=/path/to/glTF-Sample-Assets/Models \
  cargo test --locked render_twenty -- --ignored --nocapture
```

The filter runs `render_twenty_khronos_models` and
`render_twenty_more_khronos_models`, each covering 20 models. Each test evaluates
animation at four times, checks finite geometry, renders three frames per model,
and checks nonempty pixel readback and GPU validation. Results and PPM frames go
to `/tmp/tangible-gltf-samples/batch-1` and `batch-2`; override the output root with
`TANGIBLE_GLTF_OUTPUT`. Native window/UI checks are separate.

### Performance measurements

```bash
TANGIBLE_BENCH_MODELS='/path/model.stl:/path/model.glb' \
  cargo test --release --locked benchmark_models -- --ignored --nocapture
```

Use semicolons between paths on Windows. This reports parsing time, resident
geometry/texture storage, and average CPU animation evaluation time, not peak
process memory. On Linux, wrap the command with `/usr/bin/time -v` to measure
maximum resident memory. Validate rendering separately with the offscreen checks.

Run the viewer with `RUST_LOG=tangible=debug` to log background GPU preparation
and CPU frame submission durations. These include CPU preparation and queue
submission, not completed GPU execution time.
