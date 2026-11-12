mod camera;
mod loading;
mod mesh;
mod model;
mod preview;
mod renderer;
mod stl;
mod theme;
mod ui;

use std::sync::Arc;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Event, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{EventLoop, EventLoopBuilder};
use winit::keyboard::{Key, KeyCode, ModifiersState, PhysicalKey};
use winit::window::WindowBuilder;

use loading::{spawn_file_load, spawn_open_dialog, AppEvent};
use renderer::GpuState;
use ui::{format_number, plate_tile};

// ── Loaded File Info ─────────────────────────────────────────────────────────

struct FileInfo {
    units: Option<&'static str>,
    geometry_summary: String,
    dimensions: glam::Vec3,
    center: glam::Vec3,
    extent: f32,
}

// ── Entry point ──────────────────────────────────────────────────────────────

fn main() {
    env_logger::init();

    let args: Vec<String> = std::env::args().collect();
    let initial_path = if args.len() >= 2 {
        Some(std::path::PathBuf::from(&args[1]))
    } else {
        None
    };

    let (event_loop, window) = create_window();
    let proxy = event_loop.create_proxy();

    let egui_ctx = egui::Context::default();
    theme::setup_fonts(&egui_ctx);
    let cube_image = egui::ColorImage::from_rgba_unmultiplied(
        [256, 256],
        include_bytes!("../assets/impossible_cube_fuchsia.rgba"),
    );
    let cube_texture =
        egui_ctx.load_texture("impossible_cube", cube_image, egui::TextureOptions::LINEAR);

    let system_theme = window.theme().unwrap_or(winit::window::Theme::Dark);
    let mut system_is_dark = matches!(system_theme, winit::window::Theme::Dark);
    let mut theme_pref = theme::ThemePreference::System;
    let mut is_dark = theme_pref.is_dark(system_is_dark);
    theme::apply_theme(&egui_ctx, is_dark);

    let mut egui_state = egui_winit::State::new(
        egui_ctx.clone(),
        egui::ViewportId::ROOT,
        &window,
        Some(window.scale_factor() as f32),
        None,
    );

    let mut file_info: Option<FileInfo> = None;
    let mut loading_name: Option<String> = None;
    let mut error_message: Option<String> = None;
    let mut camera = camera::Camera::new(glam::Vec3::ZERO, 5.0);

    let size = window.inner_size();
    camera.aspect = size.width as f32 / size.height.max(1) as f32;

    let initial_mesh = mesh::Mesh::empty();
    let mut gpu = match GpuState::new(window.clone(), &initial_mesh) {
        Ok(gpu) => gpu,
        Err(error) => {
            rfd::MessageDialog::new()
                .set_title("Tangible could not start")
                .set_description(&error)
                .set_level(rfd::MessageLevel::Error)
                .show();
            return;
        }
    };
    gpu.set_antialiasing(true);
    let mut document: Option<Arc<model::ModelDocument>> = None;
    let mut plate_previews: Vec<Option<egui::TextureHandle>> = Vec::new();
    let mut all_preview: Option<egui::TextureHandle> = None;
    let mut selected_plate: Option<usize> = None;
    let mut plates_collapsed = false;
    let mut cube_offset = egui::Vec2::ZERO;
    let mut cube_velocity = egui::Vec2::ZERO;
    let mut cube_last_time = std::time::Instant::now();
    let mut cube_dragged = false;
    let mut cube_angle: f32 = 0.0;
    let mut cube_spin: f32 = 0.0;
    let mut cube_entered = false;

    // Load initial file in background if specified via CLI
    if let Some(path) = initial_path {
        spawn_file_load(path, proxy.clone());
    }

    // ── Interaction state ────────────────────────────────────────────────
    let mut left_pressed = false;
    let mut right_pressed = false;
    let mut last_cursor: Option<(f64, f64)> = None;
    let mut modifiers = ModifiersState::default();

    let mut should_open_file = false;
    let mut should_reset_camera = false;
    let mut should_clear_error = false;
    let mut should_update_theme = false;
    let mut requested_plate: Option<Option<usize>> = None;

    // ── Run event loop ───────────────────────────────────────────────────
    event_loop
        .run(move |event, elwt| match event {
            // ── Background Task Events ───────────────────────────────────────
            Event::UserEvent(app_event) => match app_event {
                AppEvent::LoadingStarted { id, name } => {
                    if !loading::is_current(id) { return; }
                    loading_name = Some(name);
                    error_message = None;
                    window.request_redraw();
                }
                AppEvent::MeshLoaded { id, path, document: loaded } => {
                    if !loading::is_current(id) { return; }
                    let mesh = &loaded.mesh;
                    let geometry_summary = geometry_summary(mesh);
                    let center = mesh.center();
                    let extent = mesh.extent();
                    let dims = mesh.dimensions();
                    let file_name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    camera.frame_target(center, extent);
                    gpu.load_mesh(mesh);
                    window.set_title(&format!("Tangible — {file_name}"));

                    file_info = Some(FileInfo {
                        units: loaded.units,
                        geometry_summary,
                        dimensions: dims,
                        center,
                        extent,
                    });
                    all_preview = None;
                    plate_previews = vec![None; loaded.plates.len()];
                    selected_plate = None;
                    document = Some(loaded);
                    loading_name = None;
                    error_message = None;
                    window.request_redraw();
                }
                AppEvent::PreviewReady { id, plate, image } => {
                    if !loading::is_current(id) { return; }
                    let texture = egui_ctx.load_texture(format!("preview-{id}-{plate:?}"), image, egui::TextureOptions::LINEAR);
                    if let Some(index) = plate {
                        if let Some(slot) = plate_previews.get_mut(index) { *slot = Some(texture); }
                    } else { all_preview = Some(texture); }
                    window.request_redraw();
                }
                AppEvent::LoadError { id, path, error } => {
                    if !loading::is_current(id) { return; }
                    loading_name = None;
                    error_message =
                        Some(format!("Failed to load \"{}\": {error}", path.display()));
                    window.request_redraw();
                }
            },

            // ── Window Events ────────────────────────────────────────────────
            Event::WindowEvent { event, .. } => {
                let response = egui_state.on_window_event(&window, &event);
                if response.repaint {
                    window.request_redraw();
                }

                if response.consumed {
                    if let WindowEvent::MouseInput {
                        state: ElementState::Released,
                        ..
                    } = &event
                    {
                        left_pressed = false;
                        right_pressed = false;
                    }
                    return;
                }

                match event {
                    WindowEvent::CloseRequested => elwt.exit(),

                    WindowEvent::ThemeChanged(theme) => {
                        system_is_dark = matches!(theme, winit::window::Theme::Dark);
                        let new_is_dark = theme_pref.is_dark(system_is_dark);
                        if new_is_dark != is_dark {
                            is_dark = new_is_dark;
                            theme::apply_theme(&egui_ctx, is_dark);
                        }
                        window.request_redraw();
                    }

                    WindowEvent::Resized(size) => {
                        gpu.resize(size.width, size.height);
                        camera.aspect = size.width as f32 / size.height.max(1) as f32;
                    }

                    WindowEvent::DroppedFile(path) => {
                        spawn_file_load(path, proxy.clone());
                    }

                    WindowEvent::ModifiersChanged(new_modifiers) => {
                        modifiers = new_modifiers.state();
                    }

                    WindowEvent::KeyboardInput {
                        event: key_event, ..
                    } => {
                        if key_event.state == ElementState::Pressed && !key_event.repeat {
                            let is_ctrl_or_cmd =
                                modifiers.control_key() || modifiers.super_key();
                            let is_o = match &key_event.logical_key {
                                Key::Character(s) => s.eq_ignore_ascii_case("o"),
                                _ => key_event.physical_key == PhysicalKey::Code(KeyCode::KeyO),
                            };

                            if key_event.physical_key == PhysicalKey::Code(KeyCode::KeyF) && !is_ctrl_or_cmd && !egui_ctx.wants_keyboard_input() {
                                if let Some(info) = &file_info {
                                    camera.target = info.center;
                                    camera.distance = camera.fit_distance(info.extent);
                                    window.request_redraw();
                                }
                            }
                            if is_o && (is_ctrl_or_cmd || !egui_ctx.wants_keyboard_input()) {
                                spawn_open_dialog(proxy.clone());
                            }
                        }
                    }

                    WindowEvent::MouseInput { state, button, .. } => {
                        let pressed = state == ElementState::Pressed;
                        if pressed && egui_ctx.wants_pointer_input() {
                            return;
                        }
                        match button {
                            MouseButton::Left => left_pressed = pressed,
                            MouseButton::Right | MouseButton::Middle => {
                                right_pressed = pressed
                            }
                            _ => {}
                        }
                    }

                    WindowEvent::CursorMoved { position, .. } => {
                        if let Some((lx, ly)) = last_cursor {
                            let dx = position.x - lx;
                            let dy = position.y - ly;

                            if left_pressed {
                                camera.orbit(dx as f32, dy as f32);
                                window.request_redraw();
                            }
                            if right_pressed {
                                camera.pan(dx as f32, dy as f32);
                                window.request_redraw();
                            }
                        }
                        last_cursor = Some((position.x, position.y));
                    }

                    WindowEvent::MouseWheel { delta, .. } => {
                        if egui_ctx.wants_pointer_input() {
                            return;
                        }
                        let scroll = match delta {
                            MouseScrollDelta::LineDelta(_, y) => y,
                            MouseScrollDelta::PixelDelta(p) => p.y as f32 / 50.0,
                        };
                        camera.zoom(scroll);
                        window.request_redraw();
                    }

                    WindowEvent::Focused(false) => {
                        left_pressed = false;
                        right_pressed = false;
                    }

                    WindowEvent::RedrawRequested => {
                        let raw_input = egui_state.take_egui_input(&window);
                        let mut scene_left_points = 0.0;
                        let full_output = egui_ctx.run(raw_input, |ctx| {
                            // ── Top Bar ──────────────────────────────────────────
                            egui::TopBottomPanel::top("top_panel")
                                .frame(
                                    egui::Frame::none()
                                        .fill(ctx.style().visuals.panel_fill)
                                        .inner_margin(egui::Margin::symmetric(12.0, 7.0)),
                                )
                                .show(ctx, |ui| {
                                    ui.horizontal(|ui| {
                                        if theme::radix_button(
                                            ui,
                                            Some(theme::Icon::Folder),
                                            "Open Model",
                                        )
                                        .on_hover_text("Open an STL, OBJ, or 3MF file from disk (Ctrl+O / O)")
                                        .clicked()
                                        {
                                            should_open_file = true;
                                        }

                                        if let Some(loading) = &loading_name {
                                            ui.add_space(6.0);
                                            ui.spinner();
                                            ui.label(
                                                egui::RichText::new(format!("Loading {loading}..."))
                                                    .color(if is_dark {
                                                        egui::Color32::from_rgb(147, 197, 253)
                                                    } else {
                                                        egui::Color32::from_rgb(37, 99, 235)
                                                    }),
                                            );
                                        }

                                        // Right-aligned: display settings
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if theme::theme_segmented_control(
                                                    ui,
                                                    &mut theme_pref,
                                                ) {
                                                    should_update_theme = true;
                                                }
                                                theme::projection_segmented_control(ui, &mut camera.orthographic);
                                                ui.checkbox(&mut gpu.show_edges, "Edges").on_hover_text("Show triangle edges on the shaded model");
                                                if gpu.msaa_supported {
                                                    ui.add_space(10.0);
                                                    let mut enabled = gpu.sample_count > 1;
                                                    if ui.checkbox(&mut enabled, "4× AA").on_hover_text("Smooth 3D model edges with 4× MSAA").changed() {
                                                        gpu.set_antialiasing(enabled);
                                                    }
                                                }

                                            },
                                        );
                                    });
                                });

                            if let Some(doc) = &document {
                                if !doc.plates.is_empty() {
                                    if plates_collapsed {
                                        let panel = egui::SidePanel::left("plates_panel_collapsed")
                                            .resizable(false)
                                            .exact_width(24.0)
                                            .frame(egui::Frame::none()
                                                .fill(ctx.style().visuals.panel_fill)
                                                .inner_margin(0.0))
                                            .show(ctx, |ui| {
                                                let rect = ui.max_rect();
                                                let response = ui.interact(
                                                    rect,
                                                    ui.id().with("expand_plates"),
                                                    egui::Sense::click(),
                                                ).on_hover_cursor(egui::CursorIcon::PointingHand)
                                                 .on_hover_text("Expand plates panel");
                                                ui.painter().text(
                                                    egui::pos2(rect.center().x, rect.top() + 20.0),
                                                    egui::Align2::CENTER_CENTER,
                                                    "›",
                                                    egui::TextStyle::Heading.resolve(ui.style()),
                                                    ui.visuals().text_color(),
                                                );
                                                if response.clicked() { plates_collapsed = false; }
                                            });
                                        scene_left_points = panel.response.rect.right();
                                    } else {
                                        let panel = egui::SidePanel::left("plates_panel_open")
                                            .resizable(true)
                                            .default_width(176.0)
                                            .width_range(160.0..=400.0)
                                            .frame(egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin {
                                                left: 8.0, right: 2.0, top: 2.0, bottom: 2.0,
                                            }))
                                            .show(ctx, |ui| {
                                                let (header, header_response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::click());
                                                if header_response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Minimize plates panel").clicked() {
                                                    plates_collapsed = true;
                                                }
                                                let mut header_ui = ui.child_ui(
                                                    header.shrink2(egui::vec2(6.0, 0.0)),
                                                    egui::Layout::left_to_right(egui::Align::Center),
                                                    None,
                                                );
                                                if header_ui.add(egui::Label::new(egui::RichText::new("Plates").heading()).sense(egui::Sense::click()))
                                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                                    .on_hover_text("Minimize plates panel")
                                                    .clicked()
                                                {
                                                    plates_collapsed = true;
                                                }
                                                ui.separator();
                                                ui.style_mut().spacing.scroll.floating = false;
                                                ui.style_mut().spacing.scroll.bar_inner_margin = 6.0;
                                                egui::ScrollArea::vertical().show(ui, |ui| {
                                                    if plate_tile(ui, "All plates", all_preview.as_ref(), selected_plate.is_none()) {
                                                        requested_plate = Some(None);
                                                    }
                                                    for (index, plate) in doc.plates.iter().enumerate() {
                                                        if plate_tile(ui, &plate.name, plate_previews[index].as_ref(), selected_plate == Some(index)) {
                                                            requested_plate = Some(Some(index));
                                                        }
                                                    }
                                                });
                                            });
                                        scene_left_points = panel.response.rect.right();
                                    }
                                }
                            }

                            // ── Bottom Status Bar ────────────────────────────────
                            let bottom_panel = egui::TopBottomPanel::bottom("bottom_panel")
                                .frame(
                                    egui::Frame::none()
                                        .fill(ctx.style().visuals.panel_fill)
                                        .inner_margin(egui::Margin::symmetric(12.0, 5.0)),
                                )
                                .show(ctx, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(
                                                "Left Drag: Orbit  •  Right Drag: Pan  •  Scroll: Zoom",
                                            )
                                            .small()
                                            .color(ui.visuals().widgets.noninteractive.text_color()),
                                        );

                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if let Some(info) = &file_info {
                                                    let color = ui.visuals().widgets.noninteractive.text_color();
                                                    ui.label(egui::RichText::new(format!(
                                                        "{:.1} × {:.1} × {:.1}{}",
                                                        info.dimensions.x, info.dimensions.y, info.dimensions.z,
                                                        if info.units == Some("mm") { " mm" } else { "" }
                                                    )).small().color(color));
                                                    ui.add_space(16.0);
                                                    ui.label(egui::RichText::new(&info.geometry_summary).small().color(color));
                                                }

                                            },
                                        );
                                    });
                                });

                            if let Some(error) = &error_message {
                                should_clear_error |= ui::error_popup(ctx, error, bottom_panel.response.rect.height());
                            }

                            if file_info.is_some() && error_message.is_none() {
                                ui::orientation_axes(ctx, &mut camera, bottom_panel.response.rect.height());
                            }

                            if file_info.is_some()
                                && error_message.is_none()
                                && ui::reset_view_button(ctx, bottom_panel.response.rect.height())
                            {
                                should_reset_camera = true;
                            }

                            // ── Welcome Screen (when no model loaded) ────────────
                            if file_info.is_none() {
                                egui::CentralPanel::default()
                                    .frame(egui::Frame::none().fill(if is_dark { egui::Color32::from_rgb(20, 22, 27) } else { egui::Color32::from_rgb(246, 247, 249) }))
                                    .show(ctx, |ui| {
                                        let available = ui.available_rect_before_wrap();
                                        let card_w = 380.0_f32;
                                        let card_inner = 32.0_f32;

                                        // Measure card height by running a dry layout pass
                                        let card_height = {
                                            let icon_h = 72.0_f32;
                                            let heading_h = 24.0_f32;
                                            let subtitle_h = 15.0_f32;
                                            let btn_h = 28.0_f32;
                                            let hint_h = 15.0_f32;
                                            let badge_h = 23.0_f32;
                                            let spacing = 8.0 + 4.0 + 20.0 + 14.0 + 8.0;
                                            icon_h + heading_h + subtitle_h + btn_h + hint_h + badge_h + spacing + card_inner * 2.0
                                        };

                                        // Center the card in the available area
                                        let card_x = available.center().x - card_w * 0.5 - card_inner;
                                        let card_y = (available.center().y - card_height * 0.5).max(available.top() + 16.0);

                                        ui.allocate_ui_at_rect(
                                            egui::Rect::from_min_size(
                                                egui::pos2(card_x, card_y),
                                                egui::vec2(card_w + card_inner * 2.0, card_height),
                                            ),
                                            |ui| {
                                                egui::Frame::none()
                                                    .inner_margin(card_inner)
                                                    .show(ui, |ui| {
                                                        ui.set_min_width(card_w);
                                                        ui.set_max_width(card_w);
                                                        ui.vertical_centered(|ui| {
                                                            let (slot, _) = ui.allocate_exact_size(egui::vec2(72.0, 72.0), egui::Sense::hover());
                                                            let floor = slot.bottom() + 16.0;
                                                            let radius = 51.0_f32;
                                                            let base = egui::pos2(slot.center().x, floor - radius);
                                                            ui.painter().line_segment([egui::pos2(available.left(), floor), egui::pos2(available.right(), floor)], egui::Stroke::new(1.0_f32, if is_dark { egui::Color32::from_gray(80) } else { egui::Color32::from_gray(190) }));
                                                            let dt = cube_last_time.elapsed().as_secs_f32().min(0.033);
                                                            cube_last_time = std::time::Instant::now();
                                                            if !cube_dragged {
                                                                cube_velocity.y += 1400.0 * dt;
                                                                cube_offset += cube_velocity * dt;
                                                                cube_angle += cube_spin * dt;
                                                                cube_spin *= (-0.4 * dt).exp();
                                                                if cube_offset.y >= 0.0 {
                                                                    cube_offset.y = 0.0;
                                                                    if cube_velocity.y > 35.0 {
                                                                        cube_velocity.y *= -0.48;
                                                                        cube_velocity.x *= 0.8;
                                                                        cube_spin *= 0.75;
                                                                    } else {
                                                                        cube_velocity = egui::Vec2::ZERO;
                                                                        cube_spin = 0.0;
                                                                    }
                                                                }
                                                                let min_x = available.left() + radius - base.x;
                                                                let max_x = available.right() - radius - base.x;
                                                                if cube_offset.x < min_x || cube_offset.x > max_x {
                                                                    cube_offset.x = cube_offset.x.clamp(min_x.min(max_x), max_x.max(min_x));
                                                                    cube_velocity.x *= -0.5;
                                                                }
                                                                let top = available.top() + radius - base.y;
                                                                if cube_offset.y >= top { cube_entered = true; }
                                                                if cube_entered && cube_offset.y < top {
                                                                    cube_offset.y = top.min(0.0);
                                                                    cube_velocity.y = cube_velocity.y.abs() * 0.5;
                                                                }
                                                            }
                                                            let sprite_rect = egui::Rect::from_center_size(base + cube_offset, egui::vec2(72.0, 72.0));
                                                            let response = ui.interact(sprite_rect, ui.id().with("bouncing_cube"), egui::Sense::click_and_drag());
                                                            if response.dragged() {
                                                                cube_dragged = true;
                                                                let delta = ctx.input(|i| i.pointer.delta());
                                                                cube_offset += delta;
                                                                cube_offset.y = cube_offset.y.clamp((available.top() + radius - base.y).min(0.0), 0.0);
                                                                cube_offset.x = cube_offset.x.clamp((available.left() + radius - base.x).min(0.0), (available.right() - radius - base.x).max(0.0));
                                                                cube_velocity = delta / dt.max(0.001);
                                                            } else if cube_dragged {
                                                                cube_dragged = false;
                                                            }
                                                            if response.clicked() {
                                                                let hit = response.interact_pointer_pos().unwrap_or(sprite_rect.center()) - sprite_rect.center();
                                                                cube_velocity.x += -hit.x * 10.0;
                                                                cube_velocity.y = -650.0;
                                                                cube_spin += hit.x * 0.16;
                                                            }
                                                            let center = base + cube_offset;
                                                            let mut mesh = egui::Mesh::with_texture(cube_texture.id());
                                                            let (sin, cos) = cube_angle.sin_cos();
                                                            for (x, y, u, v) in [(-36.0, -36.0, 0.0, 0.0), (36.0, -36.0, 1.0, 0.0), (36.0, 36.0, 1.0, 1.0), (-36.0, 36.0, 0.0, 1.0)] {
                                                                mesh.vertices.push(egui::epaint::Vertex { pos: center + egui::vec2(x * cos - y * sin, x * sin + y * cos), uv: egui::pos2(u, v), color: egui::Color32::WHITE });
                                                            }
                                                            mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
                                                            ui.painter().with_clip_rect(available).add(egui::Shape::mesh(mesh));
                                                            if cube_dragged || cube_offset.y < 0.0 || cube_velocity != egui::Vec2::ZERO || cube_spin.abs() > 0.01 {
                                                                ctx.request_repaint();
                                                            }
                                                            ui.add_space(32.0);

                                                            ui.heading("Tangible");
                                                            ui.add_space(4.0);
                                                            ui.label(
                                                                egui::RichText::new(
                                                                    "Inspect STL, OBJ, and 3MF models",
                                                                )
                                                                .small()
                                                                .color(ui.visuals().widgets.noninteractive.text_color()),
                                                            );
                                                            ui.add_space(20.0);

                                                            if let Some(loading) = &loading_name {
                                                                ui.spinner();
                                                                ui.add_space(8.0);
                                                                ui.label(format!("Loading {loading}..."));
                                                                ui.label(
                                                                    egui::RichText::new(
                                                                        "Parsing triangles in background...",
                                                                    )
                                                                    .small()
                                                                    .weak(),
                                                                );
                                                            } else {
                                                                if theme::radix_button(
                                                                    ui,
                                                                    Some(theme::Icon::Folder),
                                                                    "Open 3D Model",
                                                                )
                                                                .on_hover_text("Open file dialog (Ctrl+O / O)")
                                                                .clicked()
                                                                {
                                                                    should_open_file = true;
                                                                }

                                                                ui.add_space(14.0);
                                                                ui.label(
                                                                    egui::RichText::new(
                                                                        "or drag & drop a .stl, .obj, .3mf, .off, .ply, .gltf, or .glb file anywhere",
                                                                    )
                                                                    .small()
                                                                    .weak(),
                                                                );
                                                                ui.add_space(8.0);
                                                                theme::radix_badge(
                                                                    ui,
                                                                    None,
                                                                    "Shortcut: Ctrl+O / O",
                                                                );
                                                            }
                                                        });
                                                    });
                                            },
                                        );
                                    });
                            }
                        });

                        egui_state.handle_platform_output(&window, full_output.platform_output);

                        if let Some(selection) = requested_plate.take() {
                            selected_plate = selection;
                            if let (Some(doc), Some(info)) = (&document, &mut file_info) {
                                let mesh = selection.map(|index| &doc.plates[index].mesh).unwrap_or(&doc.mesh);
                                gpu.load_mesh(mesh);
                                info.geometry_summary = geometry_summary(mesh);
                                info.dimensions = mesh.dimensions();
                                info.center = mesh.center();
                                info.extent = mesh.extent();
                                camera.frame_target(info.center, info.extent);
                            }
                            window.request_redraw();
                        }

                        if should_update_theme {
                            should_update_theme = false;
                            let new_is_dark = theme_pref.is_dark(system_is_dark);
                            if new_is_dark != is_dark {
                                is_dark = new_is_dark;
                                theme::apply_theme(&egui_ctx, is_dark);
                            }
                            window.request_redraw();
                        }

                        let scene_left = (scene_left_points * full_output.pixels_per_point).round() as u32;
                        camera.aspect = (gpu.config.width.saturating_sub(scene_left).max(1)) as f32 / gpu.config.height.max(1) as f32;
                        gpu.render(
                            &camera,
                            &egui_ctx,
                            full_output.shapes,
                            &full_output.textures_delta,
                            full_output.pixels_per_point,
                            scene_left,
                            is_dark,
                        );

                        if should_open_file {
                            should_open_file = false;
                            spawn_open_dialog(proxy.clone());
                        }

                        if should_reset_camera {
                            should_reset_camera = false;
                            if let Some(info) = &file_info {
                                camera.frame_target(info.center, info.extent);
                            }
                            window.request_redraw();
                        }

                        if should_clear_error {
                            should_clear_error = false;
                            error_message = None;
                            window.request_redraw();
                        }
                    }

                    _ => {}
                }
            }

            Event::AboutToWait if egui_ctx.has_requested_repaint() || gpu.is_animated() => {
                window.request_redraw();
            }

            _ => {}
        })
        .unwrap();
}

fn create_window() -> (EventLoop<AppEvent>, Arc<winit::window::Window>) {
    let mut event_loop_builder = EventLoopBuilder::<AppEvent>::with_user_event();
    // winit 0.29 does not deliver file-drop events on Wayland. Prefer X11
    // (including XWayland) when available, unless the user chose a backend.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WINIT_UNIX_BACKEND").is_none() && std::env::var_os("DISPLAY").is_some() {
        use winit::platform::x11::EventLoopBuilderExtX11;
        event_loop_builder.with_x11();
    }
    let event_loop: EventLoop<AppEvent> = event_loop_builder.build().unwrap();

    let window = Arc::new(
        WindowBuilder::new()
            .with_title("Tangible")
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
            .build(&event_loop)
            .expect("Failed to create window"),
    );

    (event_loop, window)
}

fn geometry_summary(mesh: &mesh::Mesh) -> String {
    mesh.primitive_counts()
        .into_iter()
        .zip(["triangles", "lines", "points"])
        .filter(|(count, _)| *count > 0)
        .map(|(count, name)| format!("{} {name}", format_number(count)))
        .collect::<Vec<_>>()
        .join(" · ")
}
