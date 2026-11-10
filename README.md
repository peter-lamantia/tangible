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

Each push to `main` publishes builds for Linux and Windows on x86-64 and ARM64,
and macOS on Intel and Apple silicon, on the [Releases page](https://github.com/peter-lamantia/tangible/releases).
Download the ZIP for your platform and its SHA-256 checksum, then extract it.
Choose `x86_64` for Intel or AMD processors and `arm64` for ARM processors,
including Apple silicon. These additional architectures are built and tested in
CI; GPU compatibility still depends on the device and its drivers.

- **Windows:** Run `tangible.exe` from the extracted folder.
- **macOS:** Move `Tangible.app` to Applications and open it. The app is not
  signed or notarized, so macOS may require you to approve it in Privacy & Security.
- **Linux:** Run `./tangible` from the extracted folder. For an application menu
  entry, copy `tangible` to `~/.local/bin/`, `tangible.svg` to
  `~/.local/share/icons/hicolor/scalable/apps/`, and `tangible.desktop` to
  `~/.local/share/applications/`; ensure `~/.local/bin` is on your PATH.

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
