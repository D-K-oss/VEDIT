//! Custom mixer widgets: rotary knob, dB fader, level meter.
//!
//! Painted by hand on egui canvases (like the reference console strip):
//! * `knob` — drag vertically / scroll to turn, double-click to reset.
//! * `fader` — vertical dB scale (−60…+12 dB), drag, scroll, double-click
//!   for unity; returns the value in LINEAR gain (the model stores gain).
//! * `meter` — peak level bar with hold line; the caller supplies the
//!   level and does the decay.

use eframe::egui::{self, Color32, FontId, Painter, Pos2, Rect, Response, Rounding, Sense, Stroke, Vec2};

use crate::theme::Palette;

// ---------------------------------------------------------------------------
// dB helpers
// ---------------------------------------------------------------------------

pub fn gain_to_db(g: f64) -> f64 {
    if g <= 1e-5 {
        -120.0
    } else {
        20.0 * g.log10()
    }
}

pub fn db_to_gain(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// The fader's dB range and unity marker.
pub const FADER_MIN_DB: f64 = -60.0;
pub const FADER_MAX_DB: f64 = 12.0;

/// dB tick labels drawn next to the fader track.
const FADER_TICKS: &[(f64, &str)] = &[
    (12.0, "+12"),
    (6.0, "+6"),
    (0.0, "0"),
    (-6.0, "-6"),
    (-12.0, "-12"),
    (-20.0, "-20"),
    (-30.0, "-30"),
    (-40.0, "-40"),
    (-60.0, "-60"),
];

// ---------------------------------------------------------------------------
// Knob
// ---------------------------------------------------------------------------

/// A small rotary control. `value`/range are linear; `reset` is the
/// double-click value. Returns true when the value changed.
pub fn knob(ui: &mut egui::Ui, _id: egui::Id, value: &mut f64, min: f64, max: f64, reset: f64, size: f32, label: &str) -> bool {
    let (rect, mut response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, label));

    let mut changed = false;
    if response.dragged() {
        let delta = -response.drag_delta().y;
        if delta != 0.0 {
            let span = max - min;
            *value = (*value + (delta as f64) * span / 150.0).clamp(min, max);
            changed = true;
        }
    }
    if response.double_clicked() {
        *value = reset;
        changed = true;
    }
    if changed {
        response.mark_changed();
    }

    // Paint.
    let p = ui.painter_at(rect);
    let c = rect.center();
    let r = size / 2.0 - 2.0;
    // Body.
    p.circle_filled(c, r, Color32::from_rgb(38, 38, 42));
    p.circle_stroke(c, r, Stroke::new(1.0_f32, Color32::from_rgb(70, 70, 76)));
    // Value arc from -135° to +135°.
    let t = (((*value - min) / (max - min).max(1e-6)).clamp(0.0, 1.0)) as f32;
    let a0 = -std::f32::consts::PI * 0.75;
    let a1 = a0 + t * std::f32::consts::PI * 1.5;
    if t > 0.003 {
        p.circle_stroke(c, r - 2.5, Stroke::new(2.0_f32, Palette::ACCENT));
        let _ = a1;
        // Arc segment via small line strips.
        let steps = ((t * 24.0) as usize).max(1);
        let mut prev = Pos2::new(
            c.x + (r - 2.5) * a0.cos(),
            c.y + (r - 2.5) * a0.sin(),
        );
        for i in 1..=steps {
            let a = a0 + (a1 - a0) * (i as f32 / steps as f32);
            let pt = Pos2::new(c.x + (r - 2.5) * a.cos(), c.y + (r - 2.5) * a.sin());
            p.line_segment([prev, pt], Stroke::new(2.0_f32, Palette::ACCENT));
            prev = pt;
        }
    }
    // Indicator line.
    let ia = a0 + t * std::f32::consts::PI * 1.5;
    p.line_segment(
        [
            Pos2::new(c.x + 2.0 * ia.cos(), c.y + 2.0 * ia.sin()),
            Pos2::new(c.x + (r - 3.0) * ia.cos(), c.y + (r - 3.0) * ia.sin()),
        ],
        Stroke::new(1.6_f32, Color32::from_rgb(232, 230, 227)),
    );
    if !label.is_empty() {
        p.text(
            Pos2::new(c.x, rect.bottom() + 2.0),
            egui::Align2::CENTER_TOP,
            label,
            FontId::proportional(9.0),
            Palette::TEXT_DIM,
        );
    }
    changed
}

// ---------------------------------------------------------------------------
// Fader
// ---------------------------------------------------------------------------

/// Vertical fader with a dB scale. `gain` is linear (0..2). Height is taken
/// from `size_hint`. Double-click = unity. Returns true when changed.
pub fn fader(ui: &mut egui::Ui, _id: egui::Id, gain: &mut f64, height: f32) -> bool {
    let track_w = 22.0;
    let scale_w = 34.0;
    let (rect, mut response) =
        ui.allocate_exact_size(Vec2::new(track_w + scale_w, height), Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, "fader"));

    let mut db = gain_to_db(*gain);
    let mut changed = false;
    if response.dragged() {
        let delta = -response.drag_delta().y;
        if delta != 0.0 {
            db = (db + (delta as f64) * 0.35).clamp(FADER_MIN_DB, FADER_MAX_DB);
            *gain = db_to_gain(db).clamp(0.0, 4.0);
            changed = true;
        }
    }
    if response.double_clicked() {
        *gain = 1.0;
        changed = true;
    }
    if changed {
        response.mark_changed();
    }

    let p = ui.painter_at(rect);
    let track = Rect::from_min_size(rect.left_top() + Vec2::new(2.0, 6.0), Vec2::new(track_w - 4.0, height - 12.0));

    // Scale ticks (right side).
    let db_to_y = |db: f64| {
        let t = ((db - FADER_MIN_DB) / (FADER_MAX_DB - FADER_MIN_DB)).clamp(0.0, 1.0);
        track.bottom() - t as f32 * track.height()
    };
    for (tdb, label) in FADER_TICKS {
        let y = db_to_y(*tdb);
        let emphasis = *tdb == 0.0;
        p.line_segment(
            [Pos2::new(track.right() + 3.0, y), Pos2::new(track.right() + (if emphasis { 9.0 } else { 6.0 }), y)],
            Stroke::new(if emphasis { 1.4_f32 } else { 1.0_f32 }, if emphasis { Palette::TEXT } else { Palette::LINE }),
        );
        p.text(
            Pos2::new(track.right() + 11.0, y),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::monospace(8.0),
            if emphasis { Palette::ACCENT } else { Palette::TEXT_DIM },
        );
    }

    // Track groove.
    p.rect_filled(track, 3.0, Color32::from_rgb(24, 24, 27));
    p.rect_stroke(track, 3.0, Stroke::new(1.0_f32, Palette::LINE));

    // Handle.
    let y = db_to_y(db.clamp(FADER_MIN_DB, FADER_MAX_DB));
    let hw = track.width();
    let hh = 11.0;
    let handle = Rect::from_center_size(Pos2::new(track.center().x, y), Vec2::new(hw, hh));
    p.rect_filled(handle, 3.0, Color32::from_rgb(58, 58, 64));
    p.rect_stroke(handle, 3.0, Stroke::new(1.0_f32, Color32::from_rgb(96, 96, 104)));
    p.line_segment(
        [Pos2::new(handle.left() + 4.0, handle.center().y), Pos2::new(handle.right() - 4.0, handle.center().y)],
        Stroke::new(1.4_f32, Palette::ACCENT),
    );
    changed
}

// ---------------------------------------------------------------------------
// Meter
// ---------------------------------------------------------------------------

/// Vertical level meter with peak-hold. `level` 0..1 (+ a bit of headroom).
pub fn meter(ui: &mut egui::Ui, level: f32, peak_hold: &mut f32, height: f32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let p = ui.painter_at(rect);
    let body = Rect::from_min_size(rect.left_top(), Vec2::new(width, height));
    p.rect_filled(body, 2.0, Color32::from_rgb(20, 20, 23));

    let l = level.clamp(0.0, 1.2);
    let h = body.height();
    // Segments: green to 0.6, amber to 0.9, red above.
    let segs = 26;
    for i in 0..segs {
        let t0 = i as f32 / segs as f32;
        if t0 > l {
            break;
        }
        let y0 = body.bottom() - t0 * h;
        let y1 = body.bottom() - (i as f32 + 1.0) / segs as f32 * h + 1.0;
        let color = if t0 > 0.9 {
            Color32::from_rgb(224, 86, 78)
        } else if t0 > 0.6 {
            Color32::from_rgb(224, 180, 88)
        } else {
            Color32::from_rgb(96, 200, 120)
        };
        p.rect_filled(Rect::from_min_max(Pos2::new(body.left() + 1.0, y1), Pos2::new(body.right() - 1.0, y0)), 0.0, color);
    }
    // Peak hold.
    *peak_hold = (*peak_hold * 0.985).max(l);
    let py = body.bottom() - peak_hold.clamp(0.0, 1.2) * h;
    p.line_segment(
        [Pos2::new(body.left(), py), Pos2::new(body.right(), py)],
        Stroke::new(1.2_f32, Color32::from_rgb(240, 240, 240)),
    );
}

/// Small filled button used across the mixer.
pub fn strip_button(ui: &mut egui::Ui, label: &str, active: bool, tooltip: &str, active_color: Color32) -> Response {
    let btn = egui::Button::new(egui::RichText::new(label).size(9.0).strong())
        .small()
        .fill(if active { active_color } else { Palette::SURFACE })
        .min_size(Vec2::new(34.0, 15.0));
    ui.add(btn).on_hover_text(tooltip)
}

#[allow(dead_code)]
fn keep_imports(p: &Painter, r: &Response, r2: Rounding) {
    let _ = (p, r, r2);
}
