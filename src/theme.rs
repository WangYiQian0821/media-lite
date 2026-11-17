//! Colours, fonts and the small drawing helpers the screens share.

use eframe::egui::{
    self, epaint::PathShape, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId,
    Pos2, Rect, Response, Rounding, Sense, Stroke, Ui, Vec2,
};

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub panel: Color32,
    pub card: Color32,
    pub card_hover: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub line: Color32,
    pub ok: Color32,
    pub warn: Color32,
}

/// HillsLite 1.7.0 indicates selection with a WHITE pill on dark text 鈥?there
/// is no violet accent. We keep the field name `accent` for compatibility with
/// existing call sites, but in dark mode it is plain white, used sparingly.
pub const ACCENT: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
/// The gold used on the rating star, sampled from the 1.7.0 hero meta line.
pub const STAR: Color32 = Color32::from_rgb(0xF5, 0xC5, 0x18);

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            dark,
            bg: Color32::from_rgb(0x11, 0x1A, 0x28),
            panel: Color32::from_rgb(0x15, 0x20, 0x31),
            card: Color32::from_rgb(0x1F, 0x2A, 0x38),
            card_hover: Color32::from_rgb(0x27, 0x33, 0x44),
            text: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            dim: Color32::from_rgb(0x9A, 0xA4, 0xB2),
            accent: ACCENT,
            accent_soft: Color32::from_rgb(0x2A, 0x33, 0x44),
            line: Color32::from_rgb(0x2A, 0x33, 0x44),
            ok: Color32::from_rgb(0x4A, 0xD3, 0x91),
            warn: STAR,
        }
    } else {
        Palette {
            dark,
            bg: Color32::from_rgb(0xF6, 0xF5, 0xFB),
            panel: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            card: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            card_hover: Color32::from_rgb(0xEC, 0xEE, 0xF6),
            text: Color32::from_rgb(0x14, 0x16, 0x1C),
            dim: Color32::from_rgb(0x5B, 0x61, 0x72),
            accent: Color32::from_rgb(0x14, 0x16, 0x1C),
            accent_soft: Color32::from_rgb(0xEA, 0xEC, 0xF3),
            line: Color32::from_rgb(0xDF, 0xE2, 0xEC),
            ok: Color32::from_rgb(0x1E, 0x9E, 0x6A),
            warn: STAR,
        }
    }
}

/// Give egui a Windows-style text face when one is installed, plus a font that
/// carries Chinese characters, so titles in any script show up instead of boxes.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let candidates = [
        ("segoeui", "C:\\Windows\\Fonts\\segoeui.ttf"),
        ("segoeuisl", "C:\\Windows\\Fonts\\segoeuisl.ttf"),
    ];
    let mut main_added = false;
    for (name, path) in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert(name.to_string(), FontData::from_owned(bytes));
            if !main_added {
                fonts
                    .families
                    .entry(FontFamily::Proportional)
                    .or_default()
                    .insert(0, name.to_string());
                main_added = true;
            } else {
                fonts
                    .families
                    .entry(FontFamily::Proportional)
                    .or_default()
                    .push(name.to_string());
            }
            fonts
                .families
                .entry(FontFamily::Monospace)
                .or_default()
                .push(name.to_string());
        }
    }
    let user_font = std::env::var_os("APPDATA").map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir).join("MediaLite/msyh.ttf");
    for path in [user_font, r"C:\Windows\Fonts\Deng.ttf".into(), r"C:\Windows\Fonts\simhei.ttf".into()] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert("cjk".into(), FontData::from_owned(bytes));
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push("cjk".into());
            }
            break;
        }
    }
    let mut display = Vec::new();
    for (name, path) in [
        ("segoe-semibold", r"C:\Windows\Fonts\seguisb.ttf"),
        ("deng-bold", r"C:\Windows\Fonts\Dengb.ttf"),
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(name.into(), FontData::from_owned(bytes));
            display.push(name.to_string());
        }
    }
    display.extend(fonts.families[&FontFamily::Proportional].clone());
    fonts.families.insert(FontFamily::Name("home-display".into()), display);
    ctx.set_fonts(fonts);
}

pub fn apply_style(ctx: &egui::Context, p: &Palette) {
    let mut style = (*ctx.style()).clone();
    let mut v = if p.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = p.bg;
    v.window_fill = p.panel;
    v.extreme_bg_color = if p.dark {
        Color32::from_rgb(0x0A, 0x0B, 0x10)
    } else {
        Color32::from_rgb(0xEA, 0xEC, 0xF3)
    };
    v.faint_bg_color = p.card;
    v.override_text_color = Some(p.text);
    v.window_stroke = Stroke::new(1.0_f32, p.line);
    v.window_rounding = Rounding::same(12.0);
    v.menu_rounding = Rounding::same(10.0);
    v.selection.bg_fill = p.accent.gamma_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0_f32, p.accent);
    v.hyperlink_color = p.accent;
    v.widgets.noninteractive.bg_fill = p.card;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.line);
    v.widgets.inactive.bg_fill = p.card;
    v.widgets.inactive.weak_bg_fill = p.card;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.line);
    v.widgets.inactive.rounding = Rounding::same(8.0);
    v.widgets.hovered.bg_fill = p.card_hover;
    v.widgets.hovered.weak_bg_fill = p.card_hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.accent);
    v.widgets.hovered.rounding = Rounding::same(8.0);
    v.widgets.active.bg_fill = p.accent.gamma_multiply(0.7);
    v.widgets.active.weak_bg_fill = p.accent.gamma_multiply(0.7);
    v.widgets.active.rounding = Rounding::same(8.0);
    v.widgets.open.bg_fill = p.card_hover;
    style.visuals = v;
    style.spacing.item_spacing = Vec2::new(10.0, 10.0);
    style.spacing.button_padding = Vec2::new(12.0, 7.0);
    style.spacing.scroll = egui::style::ScrollStyle::solid();
    ctx.set_style(style);
}

// ------------------------------------------------------------------ helpers

pub fn heading(text: &str, size: f32, color: Color32) -> egui::RichText {
    egui::RichText::new(crate::i18n::t(text).to_string())
        .size(size)
        .color(color)
        .strong()
}

pub fn body(text: &str, size: f32, color: Color32) -> egui::RichText {
    egui::RichText::new(crate::i18n::t(text).to_string())
        .size(size)
        .color(color)
}

/// A label for places that paint text on the canvas, where egui wants a plain
/// string instead of styled text.
pub fn tl(text: &str) -> String {
    crate::i18n::t(text).to_string()
}

pub fn ellipsize(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

/// Shorten `text` until it fits inside `max_width` pixels, ending in an ellipsis.
///
/// Character counting is not enough: a CJK glyph is roughly twice the width of a
/// Latin one at the same size, so a 26-character title ran past the card and
/// printed over its neighbour. This measures the layout instead.
pub fn clip_px(
    painter: &egui::Painter,
    text: &str,
    size: f32,
    max_width: f32,
    color: Color32,
) -> String {
    let font = FontId::proportional(size);
    let full = painter.layout_no_wrap(text.to_string(), font.clone(), color);
    if full.size().x <= max_width {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let mut trial = out.clone();
        trial.push(ch);
        trial.push('\u{2026}');
        let g = painter.layout_no_wrap(trial, font.clone(), color);
        if g.size().x > max_width {
            break;
        }
        out.push(ch);
    }
    out.push('\u{2026}');
    out
}

/// A filled rounded card that reacts to the pointer. Returns the response.
pub fn card(ui: &mut Ui, size: Vec2, p: &Palette, hover_fill: bool, sense: Sense) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let fill = if hover_fill && response.hovered() {
        p.card_hover
    } else {
        p.card
    };
    ui.painter().rect_filled(rect, 14.0, fill);
    if response.hovered() {
        ui.painter()
            .rect_stroke(rect, 14.0, Stroke::new(1.0_f32, p.accent.gamma_multiply(0.6)));
    }
    response
}

/// Draws a poster-shaped placeholder: two soft tones and the first letter, so a
/// missing image still looks like artwork rather than a hole.
pub fn poster_placeholder(ui: &Ui, rect: Rect, seed: &str, name: &str, p: &Palette) {
    let painter = ui.painter();
    let hash = seed
        .bytes()
        .fold(1469598103934665603u64, |acc, b| {
            (acc ^ b as u64).wrapping_mul(1099511628211)
        });
    let hue = (hash % 360) as f32;
    let base = color_from_hue(hue, if p.dark { 0.32 } else { 0.55 }, if p.dark { 0.30 } else { 0.86 });
    let top = color_from_hue(
        (hue + 26.0) % 360.0,
        if p.dark { 0.36 } else { 0.60 },
        if p.dark { 0.20 } else { 0.93 },
    );
    painter.rect_filled(rect, 10.0, base);
    // A diagonal band gives the flat fill some shape.
    let band = Rect::from_min_max(
        Pos2::new(rect.left(), rect.top() + rect.height() * 0.52),
        Pos2::new(rect.right(), rect.bottom()),
    );
    painter.rect_filled(band, 10.0, top.gamma_multiply(0.85));
    let initial = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "\u{2026}".to_string());
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        initial,
        FontId::proportional((rect.width() * 0.34).clamp(20.0, 64.0)),
        Color32::from_white_alpha(if p.dark { 60 } else { 90 }),
    );
}

fn color_from_hue(hue: f32, sat: f32, light: f32) -> Color32 {
    let c = (1.0 - (2.0 * light - 1.0).abs()) * sat;
    let h = hue / 60.0;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = light - c / 2.0;
    let to = |v: f32| ((v + m).clamp(0.0, 1.0) * 255.0) as u8;
    Color32::from_rgb(to(r), to(g), to(b))
}

// --------------------------------------------------------------------- icons

fn line(painter: &egui::Painter, a: Pos2, b: Pos2, color: Color32, w: f32) {
    painter.line_segment([a, b], Stroke::new(w, color));
}

pub fn icon(ui: &Ui, kind: &str, center: Pos2, size: f32, color: Color32) {
    let p = ui.painter();
    let s = size;
    let w = (s * 0.09).max(1.4);
    match kind {
        "home" => {
            let base = Rect::from_center_size(
                Pos2::new(center.x, center.y + s * 0.22),
                Vec2::new(s * 0.62, s * 0.46),
            );
            p.rect_filled(base, 2.0, color.gamma_multiply(0.15));
            p.add(PathShape::convex_polygon(
                vec![
                    Pos2::new(center.x - s * 0.44, center.y + s * 0.12),
                    Pos2::new(center.x, center.y - s * 0.40),
                    Pos2::new(center.x + s * 0.44, center.y + s * 0.12),
                    Pos2::new(center.x + s * 0.34, center.y + s * 0.20),
                    Pos2::new(center.x, center.y - s * 0.22),
                    Pos2::new(center.x - s * 0.34, center.y + s * 0.20),
                ],
                color,
                Stroke::NONE,
            ));
            p.add(PathShape::convex_polygon(
                vec![
                    Pos2::new(center.x - s * 0.27, center.y + s * 0.22),
                    Pos2::new(center.x - s * 0.27, center.y + s * 0.42),
                    Pos2::new(center.x + s * 0.27, center.y + s * 0.42),
                    Pos2::new(center.x + s * 0.27, center.y + s * 0.22),
                ],
                color,
                Stroke::NONE,
            ));
        }
        "search" => {
            p.circle_stroke(
                Pos2::new(center.x - s * 0.08, center.y - s * 0.08),
                s * 0.26,
                Stroke::new(w + 0.4, color),
            );
            line(
                &p,
                Pos2::new(center.x + s * 0.12, center.y + s * 0.12),
                Pos2::new(center.x + s * 0.38, center.y + s * 0.38),
                color,
                w + 0.6,
            );
        }
        "gear" => {
            p.circle_stroke(center, s * 0.20, Stroke::new(w + 0.2, color));
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::TAU / 8.0;
                line(
                    &p,
                    Pos2::new(center.x + a.cos() * s * 0.30, center.y + a.sin() * s * 0.30),
                    Pos2::new(center.x + a.cos() * s * 0.42, center.y + a.sin() * s * 0.42),
                    color,
                    w,
                );
            }
        }
        "library" => {
            for (i, (dx, dy)) in [
                (-0.22, -0.22),
                (0.22, -0.22),
                (-0.22, 0.22),
                (0.22, 0.22),
            ]
            .iter()
            .enumerate()
            {
                let c = Pos2::new(center.x + dx * s, center.y + dy * s);
                let col = if i == 0 { color } else { color.gamma_multiply(0.65) };
                p.rect_filled(Rect::from_center_size(c, Vec2::splat(s * 0.28)), 3.0, col);
            }
        }
        "star" => {
            let mut pts = Vec::new();
            for k in 0..10 {
                let r = if k % 2 == 0 { s * 0.42 } else { s * 0.18 };
                let a = -std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::TAU / 10.0;
                pts.push(Pos2::new(center.x + a.cos() * r, center.y + a.sin() * r));
            }
            p.add(PathShape::convex_polygon(pts, color, Stroke::NONE));
        }
        "pro" => {
            // A medal: ring, inner disc and a small star. The original uses this
            // as the first top-bar button (a Pro badge), not a grid toggle.
            p.add(PathShape::convex_polygon(
                vec![
                    Pos2::new(center.x - s * 0.24, center.y - s * 0.46),
                    Pos2::new(center.x + s * 0.24, center.y - s * 0.46),
                    Pos2::new(center.x + s * 0.10, center.y - s * 0.10),
                    Pos2::new(center.x - s * 0.10, center.y - s * 0.10),
                ],
                color.gamma_multiply(0.7),
                Stroke::NONE,
            ));
            p.add(PathShape::convex_polygon(
                (0..12)
                    .map(|k| {
                        let r = if k % 2 == 0 { s * 0.34 } else { s * 0.15 };
                        let a = -std::f32::consts::FRAC_PI_2
                            + k as f32 * std::f32::consts::TAU / 12.0;
                        Pos2::new(center.x + a.cos() * r, center.y + s * 0.16 + a.sin() * r)
                    })
                    .collect(),
                color,
                Stroke::NONE,
            ));
        }
        "play" => {
            p.add(PathShape::convex_polygon(
                vec![
                    Pos2::new(center.x - s * 0.26, center.y - s * 0.36),
                    Pos2::new(center.x + s * 0.38, center.y),
                    Pos2::new(center.x - s * 0.26, center.y + s * 0.36),
                ],
                color,
                Stroke::NONE,
            ));
        }
        "pause" => {
            p.rect_filled(
                Rect::from_center_size(
                    Pos2::new(center.x - s * 0.16, center.y),
                    Vec2::new(s * 0.16, s * 0.72),
                ),
                2.0,
                color,
            );
            p.rect_filled(
                Rect::from_center_size(
                    Pos2::new(center.x + s * 0.16, center.y),
                    Vec2::new(s * 0.16, s * 0.72),
                ),
                2.0,
                color,
            );
        }
        "back" => {
            line(
                &p,
                Pos2::new(center.x + s * 0.18, center.y - s * 0.32),
                Pos2::new(center.x - s * 0.18, center.y),
                color,
                w + 0.6,
            );
            line(
                &p,
                Pos2::new(center.x - s * 0.18, center.y),
                Pos2::new(center.x + s * 0.18, center.y + s * 0.32),
                color,
                w + 0.6,
            );
        }
        "back-arrow" => {
            let tip = Pos2::new(center.x - s * 0.29, center.y);
            line(&p, tip, Pos2::new(center.x + s * 0.28, center.y), color, w);
            for dy in [-0.24, 0.24] {
                line(&p, tip, Pos2::new(center.x - s * 0.05, center.y + s * dy), color, w);
            }
        }
        "heart" | "heart-filled" => {
            let points = [
                (0.0, 0.34), (-0.28, 0.08), (-0.40, -0.12), (-0.36, -0.27),
                (-0.23, -0.35), (-0.10, -0.33), (0.0, -0.23), (0.10, -0.33),
                (0.23, -0.35), (0.36, -0.27), (0.40, -0.12), (0.28, 0.08),
            ].into_iter().map(|(x, y)| center + Vec2::new(x * s, y * s)).collect();
            p.add(PathShape {
                points,
                closed: true,
                fill: if kind == "heart-filled" { color } else { Color32::TRANSPARENT },
                stroke: Stroke::new(w, color).into(),
            });
        }
        "more" => {
            for dx in [-0.22, 0.0, 0.22] {
                p.circle_filled(center + Vec2::new(s * dx, 0.0), s * 0.047, color);
            }
        }
        "jump" => {
            let tip = Pos2::new(center.x + s * 0.15, center.y);
            line(&p, Pos2::new(center.x - s * 0.35, center.y), tip, color, w);
            for dy in [-0.20, 0.20] {
                line(&p, tip, Pos2::new(center.x - s * 0.05, center.y + s * dy), color, w);
            }
            line(&p, center + Vec2::new(s * 0.34, -s * 0.25), center + Vec2::new(s * 0.34, s * 0.25), color, w);
        }
        "minimize" => line(&p, center - Vec2::new(s * 0.29, 0.0), center + Vec2::new(s * 0.29, 0.0), color, w),
        "maximize" | "restore" => {
            let size = Vec2::splat(s * 0.53);
            let rect = Rect::from_center_size(center, size);
            if kind == "restore" {
                p.rect_stroke(rect.translate(Vec2::new(s * 0.13, -s * 0.13)), 1.0, Stroke::new(w, color));
            }
            p.rect_stroke(rect, 1.0, Stroke::new(w, color));
        }
        "close" => {
            let d = s * 0.25;
            for dy in [-d, d] {
                line(&p, center + Vec2::new(-d, dy), center + Vec2::new(d, -dy), color, w);
            }
        }
        "plus" => {
            line(
                &p,
                Pos2::new(center.x - s * 0.3, center.y),
                Pos2::new(center.x + s * 0.3, center.y),
                color,
                w + 0.4,
            );
            line(
                &p,
                Pos2::new(center.x, center.y - s * 0.3),
                Pos2::new(center.x, center.y + s * 0.3),
                color,
                w + 0.4,
            );
        }
        "check" => {
            line(
                &p,
                Pos2::new(center.x - s * 0.28, center.y),
                Pos2::new(center.x - s * 0.06, center.y + s * 0.24),
                color,
                w + 0.6,
            );
            line(
                &p,
                Pos2::new(center.x - s * 0.06, center.y + s * 0.24),
                Pos2::new(center.x + s * 0.3, center.y - s * 0.26),
                color,
                w + 0.6,
            );
        }
        "server" => {
            for (i, y) in [-0.26f32, 0.02, 0.30].iter().enumerate() {
                let band = Rect::from_center_size(
                    Pos2::new(center.x, center.y + y * s),
                    Vec2::new(s * 0.72, s * 0.18),
                );
                p.rect_filled(band, 3.0, color.gamma_multiply(if i == 0 { 0.9 } else { 0.55 }));
            }
        }
        "chevron" | "chevron-right" | "chevron-left" => {
            let dx = if kind == "chevron-left" { -s } else { s };
            line(
                &p,
                Pos2::new(center.x - dx * 0.10, center.y - s * 0.20),
                Pos2::new(center.x + dx * 0.14, center.y),
                color,
                w + 0.6,
            );
            line(
                &p,
                Pos2::new(center.x + dx * 0.14, center.y),
                Pos2::new(center.x - dx * 0.10, center.y + s * 0.20),
                color,
                w + 0.6,
            );
        }
        "infinity" => {
            let points = (0..=32).map(|index| {
                let angle = index as f32 * std::f32::consts::TAU / 32.0;
                center + Vec2::new(angle.sin() * s * 0.45, (angle * 2.0).sin() * s * 0.22)
            }).collect();
            p.add(egui::Shape::line(points, Stroke::new(w, color)));
        }
        "grid" => {
            let g = s * 0.34;
            for (dx, dy) in [(-g, -g), (g, -g), (-g, g), (g, g)] {
                p.rect_filled(
                    Rect::from_center_size(
                        Pos2::new(center.x + dx, center.y + dy),
                        Vec2::splat(s * 0.28),
                    ),
                    2.0,
                    color,
                );
            }
        }
        "list" => {
            for (i, dy) in [-0.24f32, 0.0, 0.24].iter().enumerate() {
                let y = center.y + dy * s;
                line(&p, Pos2::new(center.x - s * 0.30, y), Pos2::new(center.x + s * 0.30, y), color, w + 0.2);
                let _ = i;
            }
        }
        "refresh" => {
            // an open circle with an arrowhead, suggesting a reload
            let r = s * 0.28;
            let a0 = std::f32::consts::FRAC_PI_4;
            let a1 = std::f32::consts::TAU - std::f32::consts::FRAC_PI_4;
            let steps = 24;
            let arc: Vec<Pos2> = (0..=steps)
                .map(|i| {
                    let t = a0 + (a1 - a0) * (i as f32 / steps as f32);
                    Pos2::new(center.x + r * t.cos(), center.y + r * t.sin())
                })
                .collect();
            p.add(egui::Shape::line(arc, Stroke::new(w + 0.2, color)));
            let tip = Pos2::new(center.x + s * 0.28, center.y - s * 0.05);
            p.add(PathShape::convex_polygon(
                vec![
                    tip,
                    Pos2::new(tip.x + s * 0.16, tip.y - s * 0.10),
                    Pos2::new(tip.x + s * 0.04, tip.y + s * 0.14),
                ],
                color,
                Stroke::NONE,
            ));
        }
        "dot" => {
            p.circle_filled(center, s * 0.18, color);
        }
        _ => {
            p.circle_filled(center, s * 0.22, color);
        }
    }
}

/// Draw the play triangle / pause bars badge on artwork.
pub fn play_badge(ui: &Ui, rect: Rect, p: &Palette) {
    let c = rect.center();
    ui.painter()
        .circle_filled(c, 26.0, Color32::from_black_alpha(if p.dark { 150 } else { 120 }));
    icon(ui, "play", c + Vec2::new(2.0, 0.0), 24.0, Color32::WHITE);
}

pub fn progress_bar(ui: &Ui, rect: Rect, fraction: f32, p: &Palette, height: f32) {
    if fraction <= 0.0 {
        return;
    }
    let track = Rect::from_min_size(
        Pos2::new(rect.left() + 8.0, rect.bottom() - height - 8.0),
        Vec2::new(rect.width() - 16.0, height),
    );
    ui.painter().rect_filled(
        track,
        height / 2.0,
        Color32::from_black_alpha(if p.dark { 160 } else { 90 }),
    );
    let filled = Rect::from_min_size(
        track.min,
        Vec2::new(track.width() * fraction.clamp(0.0, 1.0), height),
    );
    ui.painter().rect_filled(filled, height / 2.0, p.accent);
}

pub fn pill(ui: &Ui, rect: Rect, text: &str, fill: Color32, color: Color32, size: f32) {
    ui.painter().rect_filled(rect, rect.height() / 2.0, fill);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(size),
        color,
    );
}

/// A rounded pill with a leading icon drawn at the left, used for the floating
/// nav segments and the server capsule in the 1.7.0 top bar.
pub fn icon_pill(
    ui: &Ui,
    rect: Rect,
    icon_kind: &str,
    text: &str,
    fill: Color32,
    text_color: Color32,
    icon_color: Color32,
    size: f32,
) {
    let painter = ui.painter();
    painter.rect_filled(rect, rect.height() / 2.0, fill);
    let icon_c = Pos2::new(rect.left() + rect.height() / 2.0, rect.center().y);
    icon(ui, icon_kind, icon_c, rect.height() * 0.5, icon_color);
    painter.text(
        Pos2::new(icon_c.x + rect.height() * 0.42, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(size),
        text_color,
    );
}

/// A circular icon button used in the floating top bar (search, list/grid,
/// refresh, settings). Returns the click response.
pub fn circle_icon_button(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    icon_kind: &str,
    p: &Palette,
) -> Response {
    let size = 36.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let fill = if resp.hovered() {
        p.card_hover
    } else {
        Color32::TRANSPARENT
    };
    if fill != Color32::TRANSPARENT {
        ui.painter().circle_filled(rect.center(), size / 2.0, fill);
    }
    icon(ui, icon_kind, rect.center(), size * 0.5, p.dim);
    resp
}

