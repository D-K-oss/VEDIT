//! Color grading: shared controls (wheels, scalars, LUT, presets) used by
//! both the Inspector and the Color workspace, plus the Color workspace
//! bottom page — Venturi-style: wheels left, scalars right, scopes docked
//! beside them (see scopes.rs).

use eframe::egui::{
    self, Align2, Button, Color32, DragValue, FontId, Grid, Pos2, RichText, ScrollArea,
    Slider,
};

use vedit_core::project::{Clip, TrackKind, Wheel};
use vedit_core::timeline;
use vedit_core::ColorGrade;

use crate::app::VeditApp;
use crate::theme::{panel_header, Palette};

// ---------------------------------------------------------------------------
// Shared clip-application helper (single copy for every panel)
// ---------------------------------------------------------------------------

/// Write an edited clip back onto its track (with clamping, undo, linked
/// A/V mirroring) and refresh the preview.
pub(crate) fn apply_clip(app: &mut VeditApp, track_id: u64, c: Clip, tag: &str) {
    let dur = c.duration.max(timeline::MIN_CLIP_DUR);
    let clamped_start = {
        let Some(track) = app.project.track(track_id) else { return };
        timeline::clamp_placement(track, Some(c.id), c.start.max(0.0), dur)
    };
    app.push_undo(tag);
    if let Some(track) = app.project.track_mut(track_id) {
        if let Some(dst) = track.find_clip_mut(c.id) {
            dst.start = clamped_start;
            dst.duration = dur;
            dst.speed = c.speed.clamp(0.25, 4.0);
            dst.volume = c.volume.clamp(0.0, 2.0);
            dst.opacity = c.opacity.clamp(0.0, 1.0);
            dst.muted = c.muted;
            dst.fade_in = c.fade_in.clamp(0.0, dur * 0.9);
            dst.fade_out = c.fade_out.clamp(0.0, dur * 0.9);
            dst.transition_out = c.transition_out;
            dst.keyframes = c.keyframes.clone();
            dst.text = c.text.clone();
            dst.grade = c.grade.clone();
            dst.transform = c.transform;
            dst.crop = c.crop;
            dst.blend = c.blend;
        }
        track
            .clips
            .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    }

    // Mirror the shared properties onto the linked A/V partner (opacity is
    // video-only and stays untouched on the audio half).
    if let Some((pt, pc)) = app.project.find_linked(track_id, c.id) {
        let p_clamped = match app.project.track(pt) {
            Some(t) => timeline::clamp_placement(t, Some(pc), clamped_start.max(0.0), dur),
            None => clamped_start,
        };
        if let Some(t) = app.project.track_mut(pt) {
            if let Some(dst) = t.find_clip_mut(pc) {
                dst.start = p_clamped;
                dst.duration = dur;
                dst.speed = c.speed.clamp(0.25, 4.0);
                dst.volume = c.volume.clamp(0.0, 2.0);
                dst.muted = c.muted;
                // Fades follow the pair so audio and video dissolve together.
                dst.fade_in = c.fade_in.clamp(0.0, dur * 0.9);
                dst.fade_out = c.fade_out.clamp(0.0, dur * 0.9);
            }
            t.clips
                .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
        }
    }

    app.mark_dirty();
    app.request_preview_frame();
}

// ---------------------------------------------------------------------------
// Grade building blocks (one copy each — the Inspector stacks them
// vertically, the Color page arranges them horizontally)
// ---------------------------------------------------------------------------

/// Preset row: Neutral / Black & White / Auto Balance / Reset.
pub(crate) fn presets_row(app: &mut VeditApp, ui: &mut egui::Ui, clip: &Clip, track_id: u64, locked: bool) {
    let mut grade = clip.grade.clone().unwrap_or_default();
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("PRESETS")
                .size(9.5)
                .strong()
                .color(Palette::TEXT_FAINT),
        );
        if ui.selectable_label(grade.is_neutral(), "Neutral").on_hover_text("No correction").clicked() {
            grade = ColorGrade { lut_path: grade.lut_path.clone(), ..Default::default() };
            changed = true;
        }
        if ui.selectable_label(false, "Black & White").on_hover_text("Desaturate").clicked() {
            grade.saturation = 0.0;
            changed = true;
        }
        if ui
            .selectable_label(false, "Auto Balance")
            .on_hover_text("Gray-world white balance from the preview frame")
            .clicked()
        {
            let (temp, tint) = app.auto_balance_hint();
            grade.temperature = (grade.temperature + temp).clamp(-1.0, 1.0);
            grade.tint = (grade.tint + tint).clamp(-1.0, 1.0);
            changed = true;
        }
        if !clip.grade.is_none() && ui.small_button("Reset").clicked() {
            let mut c = clip.clone();
            c.grade = None;
            apply_clip(app, track_id, c, "grade");
        }
    });
    if changed && !locked {
        let mut c = clip.clone();
        c.grade = if grade.is_neutral() { None } else { Some(grade) };
        apply_clip(app, track_id, c, "grade");
    }
}

/// Draw the four luma wheels; mutates `grade`. Returns true when changed.
pub(crate) fn wheels_row(
    ui: &mut egui::Ui,
    grade: &mut ColorGrade,
    locked: bool,
    wheel_size: f32,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        for (name, wheel) in [
            ("Shadows", &mut grade.shadows),
            ("Midtones", &mut grade.midtones),
            ("Highlights", &mut grade.highlights),
            ("Offset", &mut grade.offset),
        ] {
            color_wheel(ui, wheel, name, locked, &mut changed, wheel_size);
        }
    });
    changed
}

/// Scalar corrections + band ranges (shared grid).
pub(crate) fn grade_scalars(
    ui: &mut egui::Ui,
    grade: &mut ColorGrade,
    locked: bool,
    changed: &mut bool,
) {
    Grid::new("grade_scalars")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            scalar_row(ui, "Exposure", &mut grade.exposure, -2.0..=2.0, 0.01, locked, changed);
            scalar_row(ui, "Contrast", &mut grade.contrast, -1.0..=1.0, 0.01, locked, changed);
            scalar_row(ui, "Saturation", &mut grade.saturation, 0.0..=2.0, 0.01, locked, changed);
            scalar_row(ui, "Temperature", &mut grade.temperature, -1.0..=1.0, 0.01, locked, changed);
            scalar_row(ui, "Tint", &mut grade.tint, -1.0..=1.0, 0.01, locked, changed);
            // Band ranges.
            ui.label(RichText::new("Shadows range").small().color(Palette::TEXT_DIM));
            if ui.add_enabled(!locked, Slider::new(&mut grade.shadows_range, 0.05..=0.8).show_value(false)).changed() {
                *changed = true;
            }
            ui.end_row();
            ui.label(RichText::new("Highlights range").small().color(Palette::TEXT_DIM));
            if ui.add_enabled(!locked, Slider::new(&mut grade.highlights_range, 0.2..=0.98).show_value(false)).changed() {
                *changed = true;
            }
            ui.end_row();
        });
}

/// LUT load/clear row; mutates `grade.lut_path`.
pub(crate) fn lut_row(ui: &mut egui::Ui, grade: &mut ColorGrade, locked: bool, changed: &mut bool) {
    ui.horizontal(|ui| {
        if ui.add_enabled(!locked, Button::new("Load LUT…")).on_hover_text(".cube 3D LUT").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Choose a .cube LUT")
                .add_filter("3D LUT", &["cube"])
                .pick_file()
            {
                grade.lut_path = Some(path);
                *changed = true;
            }
        }
        if grade.lut_path.is_some() && ui.button("Clear LUT").clicked() {
            grade.lut_path = None;
            *changed = true;
        }
        if let Some(name) = grade.lut_name() {
            ui.label(RichText::new(format!("LUT: {name}")).small().color(Palette::OK));
        }
    });
}

/// Commit `grade` onto the clip unless it is neutral (then clear it).
pub(crate) fn commit_grade(app: &mut VeditApp, track_id: u64, clip: &Clip, grade: ColorGrade, locked: bool) {
    if locked {
        return;
    }
    let mut c = clip.clone();
    c.grade = if grade.is_neutral() { None } else { Some(grade) };
    apply_clip(app, track_id, c, "grade");
}

/// The stacked, full toolkit (used by the Inspector's Video tab).
pub(crate) fn grade_controls(
    app: &mut VeditApp,
    ui: &mut egui::Ui,
    track_id: u64,
    clip: &Clip,
    locked: bool,
    wheel_size: f32,
) {
    presets_row(app, ui, clip, track_id, locked);
    ui.add_space(2.0);
    let mut grade = clip.grade.clone().unwrap_or_default();
    let mut changed = wheels_row(ui, &mut grade, locked, wheel_size);
    grade_scalars(ui, &mut grade, locked, &mut changed);
    lut_row(ui, &mut grade, locked, &mut changed);
    if changed {
        commit_grade(app, track_id, clip, grade, locked);
    }
}

fn scalar_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    speed: f64,
    locked: bool,
    changed: &mut bool,
) {
    ui.label(RichText::new(label).small().color(Palette::TEXT_DIM));
    if ui.add_enabled(!locked, DragValue::new(value).speed(speed).range(range).max_decimals(2)).changed() {
        *changed = true;
    }
    ui.end_row();
}

/// One Resolve-style color wheel: hue ring + draggable chroma dot +
/// Lum / Sat mini-slider strips under it.
pub(crate) fn color_wheel(
    ui: &mut egui::Ui,
    wheel: &mut Wheel,
    name: &str,
    locked: bool,
    changed: &mut bool,
    size: f32,
) {
    let row_h = 14.0;
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(size + 10.0, size + row_h * 2.0 + 16.0),
        egui::Sense::click_and_drag(),
    );
    let p = ui.painter_at(rect);
    let center = Pos2::new(rect.center().x, rect.top() + size / 2.0 + 2.0);
    let r = size / 2.0;

    // Hue ring.
    let segments = 40;
    for i in 0..segments {
        let a0 = i as f32 / segments as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / segments as f32 * std::f32::consts::TAU;
        let hue = i as f32 / segments as f32;
        let color = Color32::from_rgb(
            (255.0 * (0.5 + 0.5 * (std::f32::consts::TAU * hue).cos())) as u8,
            (255.0 * (0.5 + 0.5 * (std::f32::consts::TAU * hue - 2.094).cos())) as u8,
            (255.0 * (0.5 + 0.5 * (std::f32::consts::TAU * hue + 2.094).cos())) as u8,
        );
        p.line_segment(
            [
                Pos2::new(center.x + r * a0.cos(), center.y + r * a0.sin()),
                Pos2::new(center.x + r * a1.cos(), center.y + r * a1.sin()),
            ],
            egui::Stroke::new(3.0_f32, color),
        );
    }
    // Face + neutral cross.
    p.circle_filled(center, r - 3.0, Color32::from_rgb(22, 22, 25));
    p.line_segment(
        [Pos2::new(center.x - 4.0, center.y), Pos2::new(center.x + 4.0, center.y)],
        egui::Stroke::new(1.0_f32, Palette::TEXT_FAINT),
    );
    p.line_segment(
        [Pos2::new(center.x, center.y - 4.0), Pos2::new(center.x, center.y + 4.0)],
        egui::Stroke::new(1.0_f32, Palette::TEXT_FAINT),
    );
    // Offset dot.
    let dx = wheel.x as f32 * (r - 5.0);
    let dy = -wheel.y as f32 * (r - 5.0);
    let active = wheel.x.abs() > 1e-3 || wheel.y.abs() > 1e-3;
    p.circle_filled(
        Pos2::new(center.x + dx, center.y + dy),
        4.0,
        if active { Palette::ACCENT } else { Palette::TEXT_DIM },
    );

    // Interaction: drag inside sets (x, y); double-click resets.
    if resp.dragged() && !locked {
        if let Some(pp) = resp.interact_pointer_pos() {
            let v = pp - center;
            if v.length() > 1.0 {
                wheel.x = (v.x / (r - 5.0)).clamp(-1.0, 1.0) as f64;
                wheel.y = (-v.y / (r - 5.0)).clamp(-1.0, 1.0) as f64;
                *changed = true;
            }
        }
    }
    if resp.double_clicked() && !locked {
        wheel.reset_chroma();
        *changed = true;
    }

    // Name.
    p.text(
        Pos2::new(center.x, rect.top() + size + 5.0),
        Align2::CENTER_TOP,
        name,
        FontId::proportional(9.5),
        if active { Palette::ACCENT } else { Palette::TEXT },
    );

    // Lum strip (L: −1..+1) + Sat strip (S: 0..2).
    let strip_y = rect.top() + size + 18.0;
    let strip_w = rect.width() - 14.0;
    let l_rect = egui::Rect::from_min_size(Pos2::new(rect.left() + 14.0, strip_y), egui::Vec2::new(strip_w, row_h));
    let s_rect = egui::Rect::from_min_size(Pos2::new(rect.left() + 14.0, strip_y + row_h + 3.0), egui::Vec2::new(strip_w, row_h));
    p.text(
        Pos2::new(rect.left(), strip_y + row_h / 2.0),
        Align2::LEFT_CENTER,
        "L",
        FontId::proportional(9.0),
        Palette::TEXT_FAINT,
    );
    p.text(
        Pos2::new(rect.left(), s_rect.top() + row_h / 2.0),
        Align2::LEFT_CENTER,
        "S",
        FontId::proportional(9.0),
        Palette::TEXT_FAINT,
    );

    let l_resp = ui.allocate_rect(l_rect, egui::Sense::click_and_drag());
    if l_resp.dragged() && !locked {
        if let Some(pp) = l_resp.interact_pointer_pos() {
            wheel.lum = ((pp.x - l_rect.left()) / l_rect.width().max(1.0) * 2.0 - 1.0).clamp(-1.0, 1.0) as f64;
            *changed = true;
        }
    }
    if l_resp.double_clicked() && !locked {
        wheel.lum = 0.0;
        *changed = true;
    }
    paint_mini_slider(ui, l_rect, ((wheel.lum + 1.0) / 2.0) as f32, Palette::INFO, wheel.lum.abs() > 1e-3);

    let s_resp = ui.allocate_rect(s_rect, egui::Sense::click_and_drag());
    if s_resp.dragged() && !locked {
        if let Some(pp) = s_resp.interact_pointer_pos() {
            wheel.sat = ((pp.x - s_rect.left()) / s_rect.width().max(1.0) * 2.0).clamp(0.0, 2.0) as f64;
            *changed = true;
        }
    }
    if s_resp.double_clicked() && !locked {
        wheel.sat = 1.0;
        *changed = true;
    }
    paint_mini_slider(ui, s_rect, (wheel.sat / 2.0) as f32, Palette::ACCENT, (wheel.sat - 1.0).abs() > 1e-3);
}

/// Tiny horizontal slider strip used by the wheel Lum/Sat rows.
fn paint_mini_slider(ui: &egui::Ui, rect: egui::Rect, t: f32, accent: Color32, active: bool) {
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 2.0, Color32::from_rgb(20, 20, 23));
    let t = t.clamp(0.0, 1.0);
    if t > 0.005 {
        p.rect_filled(
            egui::Rect::from_min_max(
                rect.left_bottom(),
                Pos2::new(rect.left() + rect.width() * t, rect.bottom()),
            ),
            2.0,
            accent.gamma_multiply(0.55),
        );
    }
    p.rect_stroke(rect, 2.0, egui::Stroke::new(1.0_f32, Palette::LINE));
    let hx = rect.left() + rect.width() * t;
    p.line_segment(
        [Pos2::new(hx, rect.top() + 1.0), Pos2::new(hx, rect.bottom() - 1.0)],
        egui::Stroke::new(1.6_f32, if active { Palette::TEXT } else { Palette::TEXT_DIM }),
    );
}

// ---------------------------------------------------------------------------
// Color workspace page (bottom panel above the timeline)
// ---------------------------------------------------------------------------

pub(crate) fn grade_page(app: &mut VeditApp, ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("grade_page")
        .resizable(true)
        .default_height(268.0)
        .min_height(180.0)
        .max_height(480.0)
        .show(ctx, |ui| {
            // Which video clip is selected?
            let sel = app
                .selected
                .and_then(|(tid, cid)| app.project.find_clip(tid, cid).cloned().map(|c| (tid, c)))
                .filter(|(tid, c)| app.project.track(*tid).map(|t| t.kind) == Some(TrackKind::Video) && !c.is_text());

            panel_header(ui, &format!(
                "COLOR  ·  {}",
                match &sel {
                    Some((_, c)) => app
                        .project
                        .asset(c.asset_id)
                        .map(|a| a.name.clone())
                        .unwrap_or_else(|| "clip".into()),
                    None => "no clip selected".into(),
                }
            ));

            match sel {
                Some((track_id, clip)) => {
                    let locked = app.project.track(track_id).map(|t| t.locked).unwrap_or(false);
                    ScrollArea::both().id_salt("grade_page_scroll").show(ui, |ui| {
                        ui.horizontal(|ui| {
                            // Left block: presets + wheels + LUT.
                            egui::Frame::default()
                                .fill(Palette::PANEL)
                                .inner_margin(egui::Margin::same(4.0))
                                .show(ui, |ui| {
                                    ui.vertical(|ui| {
                                        presets_row(app, ui, &clip, track_id, locked);
                                        let mut grade = clip.grade.clone().unwrap_or_default();
                                        let changed = wheels_row(ui, &mut grade, locked, 78.0);
                                        let mut lut_changed = false;
                                        lut_row(ui, &mut grade, locked, &mut lut_changed);
                                        if changed || lut_changed {
                                            commit_grade(app, track_id, &clip, grade, locked);
                                        }
                                    });
                                });

                            ui.separator();

                            // Right block: scalars + ranges.
                            egui::Frame::default()
                                .fill(Palette::PANEL)
                                .inner_margin(egui::Margin::same(4.0))
                                .show(ui, |ui| {
                                    let mut grade = clip.grade.clone().unwrap_or_default();
                                    let mut changed = false;
                                    grade_scalars(ui, &mut grade, locked, &mut changed);
                                    if changed {
                                        commit_grade(app, track_id, &clip, grade, locked);
                                    }
                                });
                        });
                    });
                }
                None => {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(
                            "Select a video clip on the timeline to grade it.\n\
                             The scopes on the right always measure the composed frame.",
                        )
                        .color(Palette::TEXT_FAINT),
                    );
                }
            }
        });
}
