//! VEDIT theme — dark charcoal + amber, tuned for an airy, uncluttered
//! workspace (Venturi-style: generous spacing, quiet chrome, clear
//! hierarchy between panels, cards and content).

use eframe::egui::{self, Color32, FontId, Margin, Rounding, Stroke, TextStyle, Vec2};

pub struct Palette;

#[allow(dead_code)]
impl Palette {
    // Backgrounds: three quiet depths so panels separate without borders.
    pub const BG: Color32 = Color32::from_rgb(12, 12, 14); // #0C0C0E  deepest (preview wells, timeline canvas)
    pub const PANEL: Color32 = Color32::from_rgb(19, 19, 22); // #131316  side panels
    pub const SURFACE: Color32 = Color32::from_rgb(28, 28, 32); // #1C1C20  cards, inputs
    pub const SURFACE_HOVER: Color32 = Color32::from_rgb(36, 36, 41);
    pub const SURFACE_ACTIVE: Color32 = Color32::from_rgb(46, 46, 52);
    pub const LINE: Color32 = Color32::from_rgb(40, 40, 45); // hairlines
    pub const LINE_SOFT: Color32 = Color32::from_rgb(31, 31, 35);

    // Text.
    pub const TEXT: Color32 = Color32::from_rgb(234, 232, 228);
    pub const TEXT_DIM: Color32 = Color32::from_rgb(148, 146, 140);
    pub const TEXT_FAINT: Color32 = Color32::from_rgb(104, 102, 98);

    // Accent (amber) + semantic colors.
    pub const ACCENT: Color32 = Color32::from_rgb(228, 184, 92); // #E4B85C
    pub const ACCENT_DIM: Color32 = Color32::from_rgb(150, 120, 62);
    pub const ACCENT_SOFT: Color32 = Color32::from_rgb(56, 46, 28); // amber-tinted fill
    pub const ACCENT_TEXT: Color32 = Color32::from_rgb(24, 17, 5);
    pub const DANGER: Color32 = Color32::from_rgb(226, 106, 92);
    pub const OK: Color32 = Color32::from_rgb(126, 190, 138);
    pub const INFO: Color32 = Color32::from_rgb(120, 170, 220);

    // Clip colors (fill + accent bar) — muted, kind-identifying.
    pub const CLIP_VIDEO: Color32 = Color32::from_rgb(44, 44, 51);
    pub const CLIP_VIDEO_BAR: Color32 = Color32::from_rgb(228, 184, 92);
    pub const CLIP_AUDIO: Color32 = Color32::from_rgb(31, 44, 38);
    pub const CLIP_AUDIO_BAR: Color32 = Color32::from_rgb(126, 190, 138);
    pub const CLIP_IMAGE: Color32 = Color32::from_rgb(48, 42, 34);
    pub const CLIP_IMAGE_BAR: Color32 = Color32::from_rgb(206, 168, 130);
    pub const CLIP_TITLE: Color32 = Color32::from_rgb(56, 47, 30);
    pub const CLIP_TITLE_BAR: Color32 = Color32::from_rgb(228, 184, 92);
    pub const CLIP_TEXT: Color32 = Color32::from_rgb(214, 212, 206);

    // Scope trace colors (color workspace).
    pub const SCOPE_TRACE: Color32 = Color32::from_rgb(150, 235, 170);
    pub const SCOPE_GRATICULE: Color32 = Color32::from_rgb(52, 56, 52);
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style()).clone();
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();

    v.panel_fill = Palette::PANEL;
    v.window_fill = Palette::PANEL;
    v.extreme_bg_color = Palette::BG;
    v.faint_bg_color = Color32::from_rgb(24, 24, 27);
    v.window_stroke = Stroke::new(1.0_f32, Palette::LINE);
    v.hyperlink_color = Palette::ACCENT;
    v.selection.bg_fill = Color32::from_rgba_premultiplied(104, 82, 38, 255);
    v.selection.stroke = Stroke::new(1.0_f32, Palette::ACCENT);
    v.override_text_color = Some(Palette::TEXT);

    let rounding = Rounding::same(6.0);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.rounding = rounding;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Palette::LINE_SOFT);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, Palette::TEXT);
    v.widgets.noninteractive.bg_fill = Palette::SURFACE;
    v.widgets.inactive.weak_bg_fill = Palette::SURFACE;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, Palette::TEXT);
    v.widgets.hovered.weak_bg_fill = Palette::SURFACE_HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Palette::ACCENT_DIM);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, Palette::TEXT);
    v.widgets.active.weak_bg_fill = Palette::SURFACE_ACTIVE;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, Palette::ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, Palette::TEXT);
    v.widgets.open.weak_bg_fill = Palette::SURFACE;
    v.widgets.open.bg_stroke = Stroke::new(1.0_f32, Palette::LINE);

    style.text_styles = [
        (TextStyle::Heading, FontId::proportional(16.0)),
        (TextStyle::Body, FontId::proportional(13.5)),
        (TextStyle::Button, FontId::proportional(13.0)),
        (TextStyle::Small, FontId::proportional(11.0)),
        (TextStyle::Monospace, FontId::monospace(12.0)),
    ]
    .into_iter()
    .collect();

    // Airy spacing everywhere — the core of the "not cramped" pass.
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.spacing.menu_margin = Margin::same(8.0);
    style.spacing.menu_spacing = 2.0;
    style.spacing.scroll = egui::style::ScrollStyle::solid();
    style.spacing.slider_rail_height = 3.0;
    style.spacing.indent = 16.0;

    ctx.set_style(style);
}

/// Uppercase section header used across panels (INSPECTOR · MIXER · GRADE).
pub fn panel_header(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(title.to_uppercase())
            .size(10.5)
            .strong()
            .color(Palette::TEXT_FAINT),
    );
    ui.add_space(2.0);
}

/// Monospace timecode text style helper.
pub fn mono(text: impl Into<String>, color: Color32) -> egui::RichText {
    egui::RichText::new(text)
        .monospace()
        .size(12.0)
        .color(color)
}
