/// Returns true when the user dismisses the error.
pub(crate) fn error_popup(ctx: &egui::Context, message: &str, bottom_height: f32) -> bool {
    let screen = ctx.screen_rect();
    let width = (screen.width() - 56.0).clamp(1.0, 420.0);
    let max_text_height = (screen.height() - bottom_height - 110.0).clamp(1.0, 240.0);
    let mut dismissed = false;
    egui::Area::new(egui::Id::new("load_error_popup"))
        .order(egui::Order::Foreground)
        .movable(false)
        .anchor(
            egui::Align2::RIGHT_BOTTOM,
            egui::vec2(-16.0, -bottom_height - 16.0),
        )
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.colored_label(ui.visuals().error_fg_color, "Load error");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        dismissed =
                            crate::theme::radix_button(ui, Some(crate::theme::Icon::Close), "")
                                .on_hover_text("Dismiss error")
                                .clicked();
                        if ui.button("Copy").on_hover_text("Copy full error").clicked() {
                            ui.output_mut(|output| output.copied_text = message.to_owned());
                        }
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_source(("load_error_text", message))
                    .max_height(max_text_height)
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(message).wrap());
                    });
            });
        });
    dismissed
}

pub(crate) fn format_number(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(*c);
    }
    result
}

pub(crate) fn plate_tile(
    ui: &mut egui::Ui,
    label: &str,
    texture: Option<&egui::TextureHandle>,
    selected: bool,
) -> bool {
    let width = (ui.available_width() - 8.0).clamp(16.0, 360.0);
    let image_size = width - 16.0;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, image_size + 44.0), egui::Sense::click());
    let fill = if selected {
        egui::Color32::from_rgb(30, 75, 160)
    } else if ui.visuals().dark_mode {
        egui::Color32::from_rgb(38, 41, 50)
    } else {
        egui::Color32::from_rgb(235, 238, 242)
    };
    let text_color = if selected || ui.visuals().dark_mode {
        egui::Color32::WHITE
    } else {
        egui::Color32::from_rgb(24, 26, 32)
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    let image_rect = egui::Rect::from_min_size(
        rect.min + egui::vec2(8.0, 8.0),
        egui::vec2(image_size, image_size),
    );
    let preview_bg = if ui.visuals().dark_mode {
        egui::Color32::from_rgb(32, 39, 49)
    } else {
        egui::Color32::from_rgb(235, 238, 242)
    };
    ui.painter().rect_filled(image_rect, 0.0, preview_bg);
    if let Some(texture) = texture {
        ui.painter().image(
            texture.id(),
            image_rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    } else {
        ui.painter().text(
            image_rect.center(),
            egui::Align2::CENTER_CENTER,
            "Preparing preview…",
            egui::TextStyle::Body.resolve(ui.style()),
            text_color,
        );
    }
    ui.painter().text(
        egui::pos2(rect.center().x, rect.bottom() - 20.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::TextStyle::Body.resolve(ui.style()),
        text_color,
    );
    ui.add_space(8.0);
    response.clicked()
}

pub(crate) fn orientation_axes(
    ctx: &egui::Context,
    camera: &mut crate::camera::Camera,
    bottom_height: f32,
) {
    egui::Area::new(egui::Id::new("orientation_axes"))
        .anchor(
            egui::Align2::RIGHT_BOTTOM,
            egui::vec2(-22.0, -bottom_height - 65.0),
        )
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(90.0, 90.0), egui::Sense::hover());
            let center = rect.center();
            let view = camera.view_matrix();
            let mut axes = [
                ("X", glam::Vec3::X, egui::Color32::from_rgb(235, 85, 85)),
                ("Y", glam::Vec3::Y, egui::Color32::from_rgb(80, 190, 110)),
                ("Z", glam::Vec3::Z, egui::Color32::from_rgb(85, 145, 245)),
            ]
            .map(|(label, axis, color)| (label, view.transform_vector3(axis), color));
            axes.sort_by(|a, b| a.1.z.total_cmp(&b.1.z));
            for (label, axis, color) in axes {
                let end = center + egui::vec2(axis.x, -axis.y) * 30.0;
                if ui
                    .interact(
                        egui::Rect::from_center_size(end, egui::vec2(20.0, 20.0)),
                        egui::Id::new(("axis", label)),
                        egui::Sense::click(),
                    )
                    .clicked()
                {
                    match label {
                        "X" => {
                            camera.yaw = std::f32::consts::FRAC_PI_2;
                            camera.pitch = 0.0;
                        }
                        "Y" => {
                            camera.yaw = 0.0;
                            camera.pitch = 1.569;
                        }
                        _ => {
                            camera.yaw = 0.0;
                            camera.pitch = 0.0;
                        }
                    }
                }
                let painter = ui.painter();
                painter.line_segment([center, end], egui::Stroke::new(2.5_f32, color));
                painter.circle_filled(end, 9.0, color);
                painter.text(
                    end,
                    egui::Align2::CENTER_CENTER,
                    label,
                    egui::FontId::proportional(11.0),
                    egui::Color32::WHITE,
                );
            }
            ui.painter()
                .circle_filled(center, 3.0, ui.visuals().text_color());
        });
}

pub(crate) fn reset_view_button(ctx: &egui::Context, bottom_height: f32) -> bool {
    egui::Area::new(egui::Id::new("floating_reset_view"))
        .order(egui::Order::Foreground)
        .anchor(
            egui::Align2::RIGHT_BOTTOM,
            egui::vec2(-16.0, -bottom_height - 16.0),
        )
        .show(ctx, |ui| {
            let button = egui::Button::new(
                egui::RichText::new("↺  Reset view").color(egui::Color32::from_rgb(35, 43, 54)),
            )
            .fill(egui::Color32::from_rgba_unmultiplied(248, 250, 252, 225))
            .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_gray(180)))
            .rounding(egui::Rounding::same(6.0));
            ui.add(button)
                .on_hover_text("Reset camera to center of model")
                .clicked()
        })
        .inner
}
