![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20macOS%20%7C%20Linux-blue)

# Tangible

A simple, cross-platform STL, OBJ, and 3MF file viewer written in Rust.

![Tangible UI](/assets/tangible-ui.png)

## Features

- **Cross-platform** — runs natively on Windows, macOS, and Linux
- **GUI File Opening** — native file dialog (`rfd`), top bar button, and drag-and-drop
- **GPU-accelerated** — uses `wgpu` (Vulkan / Metal / DX12)
- **STL, OBJ & 3MF** — binary and ASCII STL, polygonal OBJ, and zipped 3MF models
- **Antialiasing** — 4× MSAA enabled by default on supported GPUs
- **3MF plate selection** — shaded square previews for all plates and each individual plate, with a resizable and collapsible panel
- **Responsive loading** — background parsing and progressive plate previews; the most recently opened file takes priority
- **Model details** — displays filename, triangle/vertex counts, and bounding box dimensions

## Format support

OBJ faces may be convex or concave simple polygons; faces with more than 4,096
vertices are rejected. STL facets must contain exactly three finite vertices;
normals are derived from geometry for consistent shading.

3MF model units (micron, millimeter, centimeter, inch, foot, and meter) are
converted to millimeters, including transforms and plate dimensions. The model
start part is read from package relationships. For older archives without those
relationships, the conventional `3D/3dmodel.model` path is accepted. Relative
component paths are resolved within the archive. STL and OBJ dimensions remain
in the file's unspecified coordinate units.

To keep unusually large or malformed files from exhausting resources, input files
and total decompressed 3MF contents are limited to 512 MiB, each XML part to
256 MiB, and 3MF archives to 4,096 entries and 4,096 plates. Geometry is limited to 5 million
triangles per model and across plate meshes; component expansion is limited to
64 levels and 100,000 visits per model or plate collection. Invalid geometry and
unsupported sizes produce a loading error.

## Usage

You can launch the viewer with or without a file:

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

Each push to `main` builds native applications for Linux x86-64, macOS Apple
silicon, and Windows x86-64 and publishes them on the repository's Releases
page. Download and extract the archive for your platform to run the application.

## Source layout

- `src/main.rs`: window, application state, input, and main UI composition
- `src/renderer.rs`: GPU resources and rendering
- `src/loading.rs`: background loading, request tracking, and preview delivery
- `src/preview.rs`: cancellable CPU plate previews
- `src/mesh.rs`: shared geometry and validation
- `src/model.rs`, `src/model/obj.rs`, `src/model/three_mf.rs`, and `src/stl.rs`: format loading
- `src/ui.rs` and `src/theme.rs`: UI helpers and styling
