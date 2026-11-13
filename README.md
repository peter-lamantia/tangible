![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20macOS%20%7C%20Linux-blue)

# Tangible

A simple, cross-platform STL, OBJ, 3MF, OFF, PLY, glTF, and GLB file viewer written in Rust.

![Tangible UI](/assets/tangible-ui.png)

## Features

- **Cross-platform** — runs natively on Windows, macOS, and Linux
- **GUI File Opening** — native file dialog (`rfd`), top bar button, and drag-and-drop
- **GPU-accelerated** — uses `wgpu` (Vulkan / Metal / DX12)
- **STL, OBJ, 3MF, OFF, PLY, glTF & GLB** — geometry from common 3D model formats. glTF/GLB displays base-color, metallic/roughness, normal, occlusion, and emissive textures with separate UV sets; vertex colors; unlit materials; UV transforms; triangle/line/point primitives, skinned meshes, and morph targets. The first animation plays automatically in a loop (translation, rotation, and scale; step, linear, and cubic spline interpolation).
- **Antialiasing** — 4× MSAA enabled by default on supported GPUs
- **3MF plate selection** — shaded square previews for all plates and each individual plate, with a resizable and collapsible panel
- **Responsive loading** — background parsing and progressive plate previews; the most recently opened file takes priority
- **Model details** — displays filename, triangle/vertex counts, and bounding box dimensions
- **Projection toggle** — switch between Perspective and Orthographic using the segmented control beside the theme selector

## Usage

You can launch the viewer with or without a file:

The welcome-screen cube starts at rest; click or drag it to interact with it.

```bash
# Launch GUI directly (open files via button, dialog, or drag-and-drop)
cargo run --release

# Or launch with a file directly
cargo run --release -- path/to/model.obj
```

## Building

```bash
# Debug build
cargo build

# Release build (recommended)
cargo build --release
```

### Platform Requirements

- **Linux**: Vulkan-capable GPU + drivers (`mesa-vulkan-drivers` or proprietary)
- **macOS**: Metal-capable GPU (any Mac from 2012+)
- **Windows**: DX12 or Vulkan-capable GPU

## Releases

Each push to `main` publishes builds for Linux and Windows on x86-64 and ARM64,
and macOS on Intel and Apple silicon, on the [Releases page](https://github.com/peter-lamantia/tangible/releases).
Download the asset for your platform and its SHA-256 checksum. macOS builds are
DMG disk images; Linux and Windows builds are executables.
Choose `x86_64` for Intel or AMD processors and `arm64` for ARM processors,
including Apple silicon. These additional architectures are built and tested in
CI; GPU compatibility still depends on the device and its drivers.

- **Windows:** Run the downloaded `.exe` file.
  Release builds open only the application window; debug builds retain a console
  for diagnostic output.
  Windows MSVC builds statically link the Visual C++ runtime, so a separate
  Visual C++ Redistributable installation is not required. If an older download
  reports a missing `VCRUNTIME140.dll` or `MSVCP140.dll`, use a newer build or
  install Microsoft's [Visual C++ Redistributable](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)
  for your executable's architecture. Graphics drivers are still required.
- **macOS:** Open the `.dmg`, drag `Tangible.app` to Applications, and launch it.
  The app is not signed or notarized, so macOS may require approval in Privacy &
  Security.
- **Linux:** Run `chmod +x tangible-linux-*` and launch the downloaded executable.

Linux source builds also require the Wayland, xkbcommon, and GTK 3 development
packages (on Debian/Ubuntu: `libwayland-dev libxkbcommon-dev libgtk-3-dev`).

## Source layout

- `src/main.rs`: window, application state, input, and main UI composition
- `src/renderer.rs`: GPU resources and rendering
- `src/loading.rs`: background loading, request tracking, and preview delivery
- `src/preview.rs`: cancellable CPU plate previews
- `src/mesh.rs`: shared geometry and validation
- `src/model.rs`, `src/model/obj.rs`, `src/model/three_mf.rs`, and `src/stl.rs`: format loading
- `src/ui.rs` and `src/theme.rs`: UI helpers and styling

Model loaders build render geometry directly to reduce temporary memory use. PLY streams records from disk; glTF/GLB loads geometry buffers and referenced material textures. Large meshes still require CPU geometry and GPU storage.

Models support up to 10 million expanded triangles. Large models are uploaded in GPU buffer chunks; memory use still scales with triangle count.

Large-model performance: mesh copies share immutable vertex storage. The renderer uses indexed geometry when it reduces upload size and caches up to two recently viewed meshes of at most 32 MiB each. Plate thumbnails retain four-sample antialiasing with incremental rasterization. Release builds enable thin LTO.

## glTF regression tests

The loader tests use a small, pinned subset of
[Khronos glTF Sample Assets](https://github.com/KhronosGroup/glTF-Sample-Assets).
Run `cargo test --locked --all-targets` to check external and embedded buffers,
GLB, non-indexed geometry, interleaved attributes, and sparse accessors offline.
Fixture provenance, licenses, and coverage are in
[tests/fixtures/gltf-sample-assets/README.md](tests/fixtures/gltf-sample-assets/README.md).
Additional tests cover primitive modes, material attributes, animation, skinning, and GPU pipeline validation.

Legacy glTF 1.0 and GLB 1.0 static models are also supported, including named
resources, node transforms, and embedded textures. Legacy diffuse colors/textures
and shininess are approximated with the current material renderer; custom GLSL
techniques are not executed. Legacy animation and skinning require conversion
to glTF 2.0.

The `KHR_animation_pointer` extension supports node transforms/morph weights and
material base color, emissive, metallic, roughness, and alpha-cutoff factors
with floating-point output accessors. Other pointer targets report a specific
unsupported-target error.

glTF rendering uses approximate direct lighting, with core material texture maps,
`KHR_materials_unlit`, and `KHR_texture_transform` on base-color,
metallic/roughness, and emissive maps. Single-sided materials cull back faces;
transparent chunks render after opaque surfaces, sorted back to front with depth
writes disabled. Intersecting triangles within one chunk remain an approximation.
Authored tangent vectors and their handedness are preserved through skinning,
morphing, and mirrored transforms; meshes without tangents derive a basis from UVs.
Other material extensions, environment lighting, mipmaps, and animation selection
controls are not yet supported. Texture dimensions are limited to 8192 pixels;
animation currently evaluates and uploads geometry on the CPU, so large animated
models can be expensive.

To repeat the 20-model GPU rendering check against a local Khronos checkout:

```bash
TANGIBLE_GLTF_SAMPLES=/path/to/glTF-Sample-Assets/Models \
  cargo test --locked render_twenty_khronos_models -- --ignored --nocapture
```

Use `render_twenty_more_khronos_models` for a second set of 20 different models,
including recursive skeletons, normal/tangent mirrors, sparse accessors, URI edge
cases, and texture encoding. Use the filter `render_twenty` to run both sets.

This opt-in test loads 20 named models, evaluates animation at four times, checks
finite geometry, and renders three frames per model using the viewer's GPU
pipelines. It checks nonempty pixel readback and GPU validation, and writes
`results.tsv` and PPM frames under `batch-1` / `batch-2` in
`/tmp/tangible-gltf-samples` (override with
`TANGIBLE_GLTF_OUTPUT`). Native window/UI checks are separate from this offscreen
test.

When opening `.gltf` files, keep companion buffers (such as `.bin` files) in
their original relative locations. Missing-buffer errors identify the resource
and its base directory. Use the `.glb` variant for a bundled model file.
Load errors appear in a scrollable popup at the bottom right, with buttons to
copy the full message or dismiss it.
