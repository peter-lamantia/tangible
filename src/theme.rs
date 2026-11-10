#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemePreference {
    #[default]
    System,
    Dark,
    Light,
}

impl ThemePreference {
    pub fn is_dark(&self, system_is_dark: bool) -> bool {
        match self {
            Self::System => system_is_dark,
            Self::Dark => true,
            Self::Light => false,
        }
    }
}

/// 3D Viewport clear color matching Radix dark/light base canvas
pub fn clear_color(is_dark: bool) -> wgpu::Color {
    if is_dark {
        // Radix Zinc 1/2 Dark: #0e0f12
        wgpu::Color {
            r: 0.055,
            g: 0.059,
            b: 0.071,
            a: 1.0,
        }
    } else {
        // Radix Zinc 1/2 Light: #f6f7f9
        wgpu::Color {
            r: 0.965,
            g: 0.969,
            b: 0.976,
            a: 1.0,
        }
    }
}

/// Register Inter font family in egui
pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        "Inter-Regular".to_owned(),
        egui::FontData::from_static(include_bytes!(
            "../assets/fonts/Inter-Regular.ttf"
        )),
    );

    fonts.font_data.insert(
        "Inter-Medium".to_owned(),
        egui::FontData::from_static(include_bytes!(
            "../assets/fonts/Inter-Medium.ttf"
        )),
    );

    // Place Inter at the top of Proportional font hierarchy
    if let Some(prop) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        prop.insert(0, "Inter-Medium".to_owned());
        prop.insert(1, "Inter-Regular".to_owned());
    }

    ctx.set_fonts(fonts);
}

/// Apply Radix-inspired styling to egui context
pub fn apply_theme(ctx: &egui::Context, is_dark: bool) {
    let mut visuals = if is_dark {
        radix_dark_visuals()
    } else {
        radix_light_visuals()
    };

    // Consistent rounding: 6px (Radix medium radius)
    let radius = egui::Rounding::same(6.0);
    visuals.widgets.noninteractive.rounding = radius;
    visuals.widgets.inactive.rounding = radius;
    visuals.widgets.hovered.rounding = radius;
    visuals.widgets.active.rounding = radius;
    visuals.widgets.open.rounding = radius;

    // Window / Panel styling
    visuals.window_rounding = egui::Rounding::same(10.0);
    visuals.menu_rounding = egui::Rounding::same(6.0);

    ctx.set_visuals(visuals);

    // Refined spacing
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.window_margin = egui::Margin::same(14.0);

    // Typography sizes
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::new(17.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::new(13.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::new(12.5, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::new(11.0, egui::FontFamily::Proportional),
    );

    ctx.set_style(style);
}

fn radix_dark_visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();

    let bg_panel = egui::Color32::from_rgba_unmultiplied(18, 19, 23, 235);
    let bg_widget = egui::Color32::from_rgb(28, 30, 36);
    let bg_hover = egui::Color32::from_rgb(38, 41, 50);
    let bg_active = egui::Color32::from_rgb(48, 52, 64);

    let border = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 20);
    let border_hover = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 38);

    let text_primary = egui::Color32::from_rgb(240, 243, 246);
    let text_muted = egui::Color32::from_rgb(148, 153, 166);

    v.panel_fill = bg_panel;
    v.window_fill = bg_panel;
    v.window_stroke = egui::Stroke::new(1.0, border);
    v.window_shadow = egui::epaint::Shadow {
        offset: egui::vec2(0.0, 4.0),
        blur: 16.0,
        spread: 0.0,
        color: egui::Color32::from_black_alpha(100),
    };

    v.widgets.noninteractive.bg_fill = egui::Color32::TRANSPARENT;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, border);
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, text_muted);

    v.widgets.inactive.bg_fill = bg_widget;
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, border);
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, text_primary);

    v.widgets.hovered.bg_fill = bg_hover;
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, border_hover);
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);

    v.widgets.active.bg_fill = bg_active;
    v.widgets.active.bg_stroke = egui::Stroke::new(1.0, border_hover);
    v.widgets.active.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);

    v.widgets.open.bg_fill = bg_active;
    v.widgets.open.bg_stroke = egui::Stroke::new(1.0, border_hover);
    v.widgets.open.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);

    // Subtle accent color (Indigo/Blue 9)
    v.selection.bg_fill = egui::Color32::from_rgb(59, 130, 246);
    v.selection.stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(147, 197, 253));

    v.extreme_bg_color = egui::Color32::from_rgb(12, 13, 16);
    v
}

fn radix_light_visuals() -> egui::Visuals {
    let mut v = egui::Visuals::light();

    let bg_panel = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 240);
    let bg_widget = egui::Color32::from_rgb(246, 247, 249);
    let bg_hover = egui::Color32::from_rgb(238, 240, 244);
    let bg_active = egui::Color32::from_rgb(230, 233, 239);

    let border = egui::Color32::from_rgba_unmultiplied(0, 0, 0, 22);
    let border_hover = egui::Color32::from_rgba_unmultiplied(0, 0, 0, 42);

    let text_primary = egui::Color32::from_rgb(24, 26, 32);
    let text_muted = egui::Color32::from_rgb(115, 120, 132);

    v.panel_fill = bg_panel;
    v.window_fill = bg_panel;
    v.window_stroke = egui::Stroke::new(1.0, border);
    v.window_shadow = egui::epaint::Shadow {
        offset: egui::vec2(0.0, 4.0),
        blur: 14.0,
        spread: 0.0,
        color: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 25),
    };

    v.widgets.noninteractive.bg_fill = egui::Color32::TRANSPARENT;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, border);
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, text_muted);

    v.widgets.inactive.bg_fill = bg_widget;
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, border);
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, text_primary);

    v.widgets.hovered.bg_fill = bg_hover;
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, border_hover);
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, text_primary);

    v.widgets.active.bg_fill = bg_active;
    v.widgets.active.bg_stroke = egui::Stroke::new(1.0, border_hover);
    v.widgets.active.fg_stroke = egui::Stroke::new(1.0, text_primary);

    v.widgets.open.bg_fill = bg_active;
    v.widgets.open.bg_stroke = egui::Stroke::new(1.0, border_hover);
    v.widgets.open.fg_stroke = egui::Stroke::new(1.0, text_primary);

    // Subtle accent color (Indigo/Blue)
    v.selection.bg_fill = egui::Color32::from_rgb(37, 99, 235);
    v.selection.stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(96, 165, 250));

    v.extreme_bg_color = egui::Color32::from_rgb(255, 255, 255);
    v
}

// ── Vector Line Icons (Phosphor/Radix-inspired geometric icons) ──────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Icon {
    Folder,
    Reset,
    Sun,
    Moon,
    Monitor,
    Ruler,
    Close,
    Info,
}

pub fn draw_icon(painter: &egui::Painter, rect: egui::Rect, icon: Icon, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.3, color);
    let c = rect.center();
    let s = rect.width().min(rect.height()) * 0.5;

    match icon {
        Icon::Folder => {
            // Tabbed folder outline
            let min = c - egui::vec2(s * 0.9, s * 0.7);
            let max = c + egui::vec2(s * 0.9, s * 0.7);
            let tab_w = s * 0.7;
            let tab_h = s * 0.35;

            // Folder back tab
            painter.line_segment(
                [min, egui::pos2(min.x + tab_w, min.y)],
                stroke,
            );
            painter.line_segment(
                [egui::pos2(min.x + tab_w, min.y), egui::pos2(min.x + tab_w + s * 0.25, min.y + tab_h)],
                stroke,
            );
            // Folder body
            let body_rect = egui::Rect::from_min_max(
                egui::pos2(min.x, min.y + tab_h),
                max,
            );
            painter.rect_stroke(body_rect, egui::Rounding::same(2.0), stroke);
        }
        Icon::Reset => {
            // Circular reload arrow
            let r = s * 0.75;
            // Draw 270 deg circle arc using points
            let mut points = Vec::new();
            let steps = 14;
            let start_angle = std::f32::consts::PI * 0.2;
            let end_angle = std::f32::consts::PI * 1.8;
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let angle = start_angle + t * (end_angle - start_angle);
                points.push(c + egui::vec2(angle.cos() * r, angle.sin() * r));
            }
            painter.add(egui::Shape::line(points, stroke));

            // Arrow head
            let tip = c + egui::vec2(start_angle.cos() * r, start_angle.sin() * r);
            let a1 = tip + egui::vec2(-s * 0.35, -s * 0.1);
            let a2 = tip + egui::vec2(-s * 0.05, s * 0.35);
            painter.line_segment([tip, a1], stroke);
            painter.line_segment([tip, a2], stroke);
        }
        Icon::Sun => {
            // Sun: Center circle + 8 tick rays
            let r = s * 0.42;
            painter.circle_stroke(c, r, stroke);
            for i in 0..8 {
                let angle = i as f32 * std::f32::consts::PI * 0.25;
                let dir = egui::vec2(angle.cos(), angle.sin());
                let p1 = c + dir * (r + s * 0.2);
                let p2 = c + dir * (r + s * 0.45);
                painter.line_segment([p1, p2], stroke);
            }
        }
        Icon::Moon => {
            // Crescent moon: mathematically exact continuous polygon where outer and
            // inner arcs meet seamlessly at the sharp tips (zero gap, zero glitch).
            let mut points = Vec::with_capacity(30);
            let steps = 14;

            // Outer arc: sweeps from top tip to bottom tip through right side
            let outer_c = c - egui::vec2(s * 0.15, 0.0);
            let outer_r = s * 0.70;
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let angle = -std::f32::consts::FRAC_PI_2 + t * std::f32::consts::PI;
                points.push(outer_c + egui::vec2(angle.cos() * outer_r, angle.sin() * outer_r));
            }

            // Inner arc: sweeps from bottom tip back to top tip
            let inner_c = c - egui::vec2(s * 1.005, 0.0);
            let inner_r = s * 1.105;
            let tip_angle = 0.686_f32; // atan2(0.70, 0.855)
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let angle = tip_angle - t * (2.0 * tip_angle);
                points.push(inner_c + egui::vec2(angle.cos() * inner_r, angle.sin() * inner_r));
            }

            painter.add(egui::Shape::closed_line(points, stroke));
        }
        Icon::Monitor => {
            // Screen outline
            let screen = egui::Rect::from_center_size(
                c - egui::vec2(0.0, s * 0.15),
                egui::vec2(s * 1.7, s * 1.15),
            );
            painter.rect_stroke(screen, egui::Rounding::same(2.0), stroke);
            // Stand
            let stand_top = egui::pos2(c.x, screen.bottom());
            let stand_bottom = stand_top + egui::vec2(0.0, s * 0.35);
            painter.line_segment([stand_top, stand_bottom], stroke);
            let base_l = stand_bottom - egui::vec2(s * 0.45, 0.0);
            let base_r = stand_bottom + egui::vec2(s * 0.45, 0.0);
            painter.line_segment([base_l, base_r], stroke);
        }
        Icon::Ruler => {
            // Bounding box / dimension icon
            let rect_box = egui::Rect::from_center_size(c, egui::vec2(s * 1.5, s * 1.3));
            painter.rect_stroke(rect_box, egui::Rounding::same(2.0), stroke);
            // Dimension tick lines
            painter.line_segment(
                [
                    egui::pos2(rect_box.left() + s * 0.4, rect_box.top()),
                    egui::pos2(rect_box.left() + s * 0.4, rect_box.top() + s * 0.35),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(rect_box.left() + s * 0.8, rect_box.top()),
                    egui::pos2(rect_box.left() + s * 0.8, rect_box.top() + s * 0.35),
                ],
                stroke,
            );
        }
        Icon::Close => {
            let offset = s * 0.6;
            painter.line_segment(
                [c - egui::vec2(offset, offset), c + egui::vec2(offset, offset)],
                stroke,
            );
            painter.line_segment(
                [c + egui::vec2(-offset, offset), c + egui::vec2(offset, -offset)],
                stroke,
            );
        }
        Icon::Info => {
            painter.circle_stroke(c, s * 0.75, stroke);
            let dot = c - egui::vec2(0.0, s * 0.35);
            painter.circle_filled(dot, 1.2, color);
            let stem_top = c - egui::vec2(0.0, s * 0.1);
            let stem_bot = c + egui::vec2(0.0, s * 0.4);
            painter.line_segment([stem_top, stem_bot], stroke);
        }
    }
}

/// Radix-styled button with an integrated vector icon
pub fn radix_button(
    ui: &mut egui::Ui,
    icon: Option<Icon>,
    label: &str,
) -> egui::Response {
    let font_id = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        font_id,
        ui.visuals().widgets.inactive.text_color(),
    );

    let icon_size = 14.0;
    let icon_gap = if icon.is_some() && !label.is_empty() { 6.0 } else { 0.0 };
    let has_icon = icon.is_some();

    let content_w = (if has_icon { icon_size } else { 0.0 })
        + icon_gap
        + galley.size().x;
    let content_h = galley.size().y.max(icon_size);

    let padding = ui.style().spacing.button_padding;
    let desired_size = egui::vec2(content_w + padding.x * 2.0, content_h + padding.y * 2.0);

    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);

        // Background & border
        ui.painter().rect(
            rect,
            visuals.rounding,
            visuals.bg_fill,
            visuals.bg_stroke,
        );

        let mut start_x = rect.min.x + padding.x;
        let center_y = rect.center().y;

        // Draw icon
        if let Some(ic) = icon {
            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(start_x + icon_size * 0.5, center_y),
                egui::vec2(icon_size, icon_size),
            );
            draw_icon(ui.painter(), icon_rect, ic, visuals.text_color());
            start_x += icon_size + icon_gap;
        }

        // Draw label
        if !label.is_empty() {
            let text_pos = egui::pos2(
                start_x,
                center_y - galley.size().y * 0.5,
            );
            ui.painter().galley(text_pos, galley, visuals.text_color());
        }
    }

    response
}

/// Radix-styled compact metadata badge / chip
pub fn radix_badge(ui: &mut egui::Ui, icon: Option<Icon>, text: &str) {
    let is_dark = ui.visuals().dark_mode;
    let bg_color = if is_dark {
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 12)
    } else {
        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 10)
    };
    let border_color = if is_dark {
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 22)
    } else {
        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 20)
    };
    let text_color = if is_dark {
        egui::Color32::from_rgb(195, 200, 212)
    } else {
        egui::Color32::from_rgb(70, 75, 85)
    };

    let font_id = egui::TextStyle::Small.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font_id, text_color);

    let icon_size = 12.0;
    let icon_gap = if icon.is_some() { 5.0 } else { 0.0 };
    let total_w = (if icon.is_some() { icon_size } else { 0.0 }) + icon_gap + galley.size().x;
    let padding = egui::vec2(8.0, 4.0);

    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(total_w + padding.x * 2.0, galley.size().y.max(icon_size) + padding.y * 2.0),
        egui::Sense::hover(),
    );

    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            egui::Rounding::same(12.0), // pill badge
            bg_color,
            egui::Stroke::new(1.0, border_color),
        );

        let mut start_x = rect.min.x + padding.x;
        let center_y = rect.center().y;

        if let Some(ic) = icon {
            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(start_x + icon_size * 0.5, center_y),
                egui::vec2(icon_size, icon_size),
            );
            draw_icon(ui.painter(), icon_rect, ic, text_color);
            start_x += icon_size + icon_gap;
        }

        ui.painter().galley(
            egui::pos2(start_x, center_y - galley.size().y * 0.5),
            galley,
            text_color,
        );
    }
}

/// Radix Segmented Theme Selector: [ Monitor System | Sun Light | Moon Dark ]
pub fn theme_segmented_control(
    ui: &mut egui::Ui,
    current: &mut ThemePreference,
) -> bool {
    let options = [
        (ThemePreference::System, Icon::Monitor, "System"),
        (ThemePreference::Light, Icon::Sun, "Light"),
        (ThemePreference::Dark, Icon::Moon, "Dark"),
    ];

    let is_dark = ui.visuals().dark_mode;
    let container_bg = if is_dark {
        egui::Color32::from_rgb(22, 23, 27)
    } else {
        egui::Color32::from_rgb(240, 241, 244)
    };
    let border_color = if is_dark {
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 18)
    } else {
        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 18)
    };

    let font_id = egui::TextStyle::Small.resolve(ui.style());
    let icon_size = 12.0;
    let icon_gap = 4.0;
    let padding = egui::vec2(8.0, 4.0);
    let container_margin = 2.0;
    let item_spacing = 2.0;

    // Pre-layout text and compute widths
    let mut item_widths = Vec::with_capacity(3);
    let mut max_content_h: f32 = icon_size;
    let mut galleys = Vec::with_capacity(3);

    for (pref, _, label) in &options {
        let is_active = *current == *pref;
        let text_color = if is_active {
            if is_dark { egui::Color32::WHITE } else { egui::Color32::BLACK }
        } else if is_dark {
            egui::Color32::from_rgb(140, 145, 155)
        } else {
            egui::Color32::from_rgb(120, 125, 135)
        };

        let galley = ui.painter().layout_no_wrap((*label).to_owned(), font_id.clone(), text_color);
        max_content_h = max_content_h.max(galley.size().y);
        let item_w = icon_size + icon_gap + galley.size().x + padding.x * 2.0;
        item_widths.push(item_w);
        galleys.push(galley);
    }

    let item_h = max_content_h + padding.y * 2.0;
    let total_w: f32 = container_margin * 2.0
        + item_widths.iter().sum::<f32>()
        + (options.len().saturating_sub(1) as f32) * item_spacing;
    let total_h = item_h + container_margin * 2.0;

    // Allocate exact rect in parent UI (handles right_to_left seamlessly as a single block)
    let (container_rect, _) = ui.allocate_exact_size(
        egui::vec2(total_w, total_h),
        egui::Sense::hover(),
    );

    let mut changed = false;

    if ui.is_rect_visible(container_rect) {
        // Draw container background and subtle border
        ui.painter().rect(
            container_rect,
            egui::Rounding::same(6.0),
            container_bg,
            egui::Stroke::new(1.0, border_color),
        );

        // Always render items left-to-right within the container
        let mut cur_x = container_rect.min.x + container_margin;
        let cur_y = container_rect.min.y + container_margin;

        for (idx, (pref, icon, _)) in options.iter().enumerate() {
            let item_w = item_widths[idx];
            let item_rect = egui::Rect::from_min_size(
                egui::pos2(cur_x, cur_y),
                egui::vec2(item_w, item_h),
            );

            let id = ui.id().with("theme_seg_btn").with(idx);
            let resp = ui.interact(item_rect, id, egui::Sense::click());
            let is_active = *current == *pref;
            let hovered = resp.hovered();

            if resp.clicked() && !is_active {
                *current = *pref;
                changed = true;
            }

            let rounding = egui::Rounding::same(4.0);
            if is_active {
                let active_bg = if is_dark {
                    egui::Color32::from_rgb(38, 41, 50)
                } else {
                    egui::Color32::WHITE
                };
                let active_stroke = if is_dark {
                    egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 30))
                } else {
                    egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 25))
                };
                ui.painter().rect(item_rect, rounding, active_bg, active_stroke);
            } else if hovered {
                let hover_bg = if is_dark {
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 8)
                } else {
                    egui::Color32::from_rgba_unmultiplied(0, 0, 0, 8)
                };
                ui.painter().rect(item_rect, rounding, hover_bg, egui::Stroke::NONE);
            }

            let text_color = if is_active {
                if is_dark { egui::Color32::WHITE } else { egui::Color32::BLACK }
            } else if is_dark {
                egui::Color32::from_rgb(140, 145, 155)
            } else {
                egui::Color32::from_rgb(120, 125, 135)
            };

            let start_x = item_rect.min.x + padding.x;
            let center_y = item_rect.center().y;

            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(start_x + icon_size * 0.5, center_y),
                egui::vec2(icon_size, icon_size),
            );
            draw_icon(ui.painter(), icon_rect, *icon, text_color);

            let galley = galleys[idx].clone();
            ui.painter().galley(
                egui::pos2(start_x + icon_size + icon_gap, center_y - galley.size().y * 0.5),
                galley,
                text_color,
            );

            cur_x += item_w + item_spacing;
        }
    }

    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_top_bar_rendering() {
        let ctx = egui::Context::default();
        setup_fonts(&ctx);
        apply_theme(&ctx, true);

        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 720.0))),
            ..Default::default()
        };

        let mut theme_pref = ThemePreference::System;

        let full_output = ctx.run(raw_input, |ctx| {
            egui::TopBottomPanel::top("top_panel")
                .frame(
                    egui::Frame::none()
                        .fill(ctx.style().visuals.panel_fill)
                        .inner_margin(egui::Margin::symmetric(12.0, 7.0)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        radix_button(ui, Some(Icon::Folder), "Open STL");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            theme_segmented_control(ui, &mut theme_pref);
                        });
                    });
                });
        });

        let primitives = ctx.tessellate(full_output.shapes, 1.0);
        assert!(!primitives.is_empty());
        for prim in &primitives {
            if let egui::ClippedPrimitive { primitive: egui::epaint::Primitive::Mesh(mesh), .. } = prim {
                for v in &mesh.vertices {
                    assert!(!v.pos.x.is_nan() && !v.pos.y.is_nan());
                }
            }
        }
    }

    #[test]
    fn test_all_icons_draw() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let icons = [
                    Icon::Folder,
                    Icon::Reset,
                    Icon::Sun,
                    Icon::Moon,
                    Icon::Monitor,
                    Icon::Ruler,
                    Icon::Close,
                    Icon::Info,
                ];
                for icon in icons {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                    draw_icon(ui.painter(), rect, icon, egui::Color32::WHITE);
                }
            });
        });
    }

    #[test]
    fn test_theme_preference_resolution() {
        let pref_system = ThemePreference::System;
        assert!(pref_system.is_dark(true));
        assert!(!pref_system.is_dark(false));

        let pref_light = ThemePreference::Light;
        assert!(!pref_light.is_dark(true));
        assert!(!pref_light.is_dark(false));

        let pref_dark = ThemePreference::Dark;
        assert!(pref_dark.is_dark(true));
        assert!(pref_dark.is_dark(false));
    }
}

