# STL Viewer

A simple, cross-platform STL file viewer written in Rust.

![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20macOS%20%7C%20Linux-blue)

## Features

- **Cross-platform** — runs natively on Windows, macOS, and Linux
- **GUI File Opening** — native file dialog (`rfd`), top bar button, and drag-and-drop
- **Launch with or without files** — shows a welcome screen when launched directly
- **GPU-accelerated** — uses `wgpu` (Vulkan / Metal / DX12)
- **Binary & ASCII STL** — auto-detects format with auto-normal calculation
- **Orbit camera** — rotate, pan, and zoom with mouse
- **Blinn-Phong lighting** — two-sided shading
- **Model details** — displays filename, triangle/vertex counts, and bounding box dimensions

## Controls

| Input                  | Action                        |
|------------------------|-------------------------------|
| **📂 Open STL Button** | Open native file picker       |
| **Drag & Drop**        | Drop any `.stl` into window   |
| `Ctrl + O` / `Cmd + O` | Open file picker dialog       |
| `O` key                | Open file picker dialog       |
| **🔄 Reset View**      | Re-center camera on model     |
| Left mouse drag        | Orbit (rotate around model)   |
| Right mouse drag       | Pan                           |
| Middle mouse drag      | Pan                           |
| Scroll wheel           | Zoom in / out                 |

## Usage

You can launch the viewer with or without a file:

```bash
# Launch GUI directly (open files via button, dialog, or drag-and-drop)
cargo run --release

# Or launch with a file directly
cargo run --release -- path/to/model.stl
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

## Dependencies

| Crate        | Purpose                                    |
|--------------|--------------------------------------------|
| `wgpu`       | Cross-platform GPU rendering               |
| `winit`      | Cross-platform windowing and event handling|
| `rfd`        | Native OS open file dialogs                |
| `egui`       | Immediate-mode GUI overlay                 |
| `glam`       | Linear algebra (vectors, matrices)         |
| `bytemuck`   | Safe casting for GPU buffers               |
| `pollster`   | Synchronous bridge for async wgpu setup    |
