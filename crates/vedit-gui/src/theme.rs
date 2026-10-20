//! Minimalist dark theme for VEDIT.

use eframe::egui::{self, Color32, FontId, Rounding, Stroke, TextStyle, Vec2};

pub struct Palette;

#[allow(dead_code)]
impl Palette {
    pub const BG: Color32 = Color32::from_rgb(14, 14, 16); // #0E0E10
    pub const PANEL: Color32 = Color32::from_rgb(21, 21, 23); // #151517
    pub const SURFACE: Color32 = Color32::from_rgb(30, 30, 33); // #1E1E21
    pub const SURFACE_HOVER: Color32 = Color32::from_rgb(38, 38, 42);
    pub const SURFACE_ACTIVE: Color32 = Color32::from_rgb(48, 48, 53);
    pub const LINE: Color32 = Color32::from_rgb(42, 42, 46);
    pub const TEXT: Color32 = Color32::from_rgb(232, 230, 227);
    pub const TEXT_DIM: Color32 = Color32::from_rgb(150, 148, 141);
    pub const ACCENT: Color32 = Color32::from_rgb(224, 180, 88); // #E0B458
    pub const ACCENT_DIM: Color32 = Color32::from_rgb(150, 120, 62);
    pub const ACCENT_TEXT: Color32 = Color32::from_rgb(26, 18, 5);
    pub const DANGER: Color32 = Color32::from_rgb(224, 108, 90);
    pub const OK: Color32 = Color32::from_rgb(127, 176, 138);

    // Clip colors (fill + accent bar)
    pub const CLIP_VIDEO: Color32 = Color32::from_rgb(41, 41, 47);
    pub const CLIP_VIDEO_BAR: Color32 = Color32::from_rgb(224, 180, 88);
    pub const CLIP_AUDIO: Color32 = Color32::from_rgb(33, 41, 37);
    pub const CLIP_AUDIO_BAR: Color32 = Color32::from_rgb(127, 176, 138);
    pub const CLIP_IMAGE: Color32 = Color32::from_rgb(45, 40, 34);
    pub const CLIP_IMAGE_BAR: Color32 = Color32::from_rgb(201, 162, 126);
    pub const CLIP_TITLE: Color32 = Color32::from_rgb(52, 44, 30);
    pub const CLIP_TITLE_BAR: Color32 = Color32::from_rgb(224, 180, 88);
    pub const CLIP_TEXT: Color32 = Color32::from_rgb(210, 208, 202);
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style()).clone();
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();

    v.panel_fill = Palette::PANEL;
    v.window_fill = Palette::PANEL;
    v.extreme_bg_color = Palette::BG;
    v.faint_bg_color = Color32::from_rgb(26, 26, 29);
    v.window_stroke = Stroke::new(1.0_f32, Palette::LINE);
    v.hyperlink_color = Palette::ACCENT;
    v.selection.bg_fill = Color32::from_rgba_premultiplied(96, 78, 38, 255);
    v.selection.stroke = Stroke::new(1.0_f32, Palette::ACCENT);

    let rounding = Rounding::same(4.0);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.rounding = rounding;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Palette::LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, Palette::TEXT);
    v.widgets.noninteractive.bg_fill = Palette::SURFACE;
    v.widgets.inactive.weak_bg_fill = Palette::SURFACE;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Palette::LINE);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, Palette::TEXT);
    v.widgets.hovered.weak_bg_fill = Palette::SURFACE_HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Palette::ACCENT_DIM);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, Palette::TEXT);
    v.widgets.active.weak_bg_fill = Palette::SURFACE_ACTIVE;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, Palette::ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, Palette::TEXT);

    style.text_styles = [
        (TextStyle::Heading, FontId::proportional(16.0)),
        (TextStyle::Body, FontId::proportional(13.0)),
        (TextStyle::Button, FontId::proportional(13.0)),
        (TextStyle::Small, FontId::proportional(11.0)),
        (TextStyle::Monospace, FontId::monospace(12.0)),
    ]
    .into_iter()
    .collect();

    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(10.0, 5.0);
    style.spacing.menu_margin = egui::Margin::same(6.0);
    style.spacing.scroll = egui::style::ScrollStyle::solid();

    ctx.set_style(style);
}

/// Monospace timecode text style helper.
pub fn mono(text: impl Into<String>, color: Color32) -> egui::RichText {
    egui::RichText::new(text)
        .monospace()
        .size(12.0)
        .color(color)
}
