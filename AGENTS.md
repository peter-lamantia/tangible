# Agent guide

Tangible is a Rust desktop viewer for STL, OBJ, and 3MF models. Keep changes focused and preserve existing user changes.

## Project map

- `src/main.rs`: window, application state, GPU rendering, and UI.
- `src/model.rs`: OBJ and 3MF loading, including plate metadata.
- `src/stl.rs`: STL loading and mesh representation.
- `src/camera.rs`: camera behavior; `src/shader.wgsl`: rendering shader.
- `src/theme.rs`: UI theme and controls; `assets/` contains bundled artwork and fonts. See `assets/ARTWORK.md` for attribution.

## Development and checks

Use stable Rust. Run `cargo build --locked --all-targets` and `cargo test --locked --all-targets` for changes that affect code. Tests live alongside the implementation. On Linux, CI installs `libwayland-dev`, `libxkbcommon-dev`, and `libgtk-3-dev`; a working graphics environment is needed to run the GUI.

CI runs on Linux, macOS, and Windows. Keep platform-specific changes portable. The release workflow builds archives on pushes to `main` and publishes them under commit-specific tags. Update the README when user-facing behavior or distribution changes.
