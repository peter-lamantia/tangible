pub(crate) fn format_number(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i) % 3 == 0 {
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
