//! Timeline panel: ruler, tracks, clips, playhead, drag/trim/snap, drops.
//! v0.2.1: waveform + filmstrip rendering inside clips, fade triangles,
//! cross-dissolve markers, per-track Solo, undo/redo + title buttons.

use eframe::egui::{
    self, Align2, Button, Color32, CursorIcon, DragAndDrop, FontId, Pos2, Rect, RichText, Sense,
    Shape, Slider, Stroke, TopBottomPanel, Vec2,
};

use vedit_core::media::MediaKind;
use vedit_core::project::TrackKind;
use vedit_core::timeline::{self, snap_time};

use crate::app::{StripTex, VeditApp};
use crate::theme::{panel_header, Palette};

const HEADER_W: f32 = 132.0;
const RULER_H: f32 = 26.0;
const MIN_LANE_H: f32 = 22.0;
const MAX_LANE_H: f32 = 52.0;
const EDGE_PX: f32 = 7.0;
const SNAP_TOL_PX: f32 = 9.0;

#[derive(Debug, Clone)]
pub struct TimelineUiState {
    pub px_per_sec: f32,
    pub scroll_x: f32,
    pub snap: bool,
    pub drag: Option<DragState>,
}

impl TimelineUiState {
    pub(crate) fn new() -> Self {
        Self {
            px_per_sec: 60.0,
            scroll_x: 0.0,
            snap: true,
            drag: None,
        }
    }
}

impl Default for TimelineUiState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub enum DragState {
    Scrub,
    Move { track_id: u64, clip_id: u64, grab_offset: f64 },
    TrimStart { track_id: u64, clip_id: u64 },
    TrimEnd { track_id: u64, clip_id: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Edge {
    None,
    Start,
    End,
}

struct ClipDraw {
    track_idx: usize,
    track_id: u64,
    id: u64,
    start: f64,
    dur: f64,
    speed: f64,
    in_point: f64,
    asset_id: u64,
    kind: Option<MediaKind>,
    is_text: bool,
    text_label: String,
    fade_in: f64,
    fade_out: f64,
    transition_d: f64,
    muted: bool,
    locked: bool,
    selected: bool,
    fill: Color32,
    bar: Color32,
    label: String,
}

struct Hit {
    track_id: u64,
    clip_id: u64,
    edge: Edge,
}

// ---------------------------------------------------------------------------

pub(crate) fn timeline(app: &mut VeditApp, ctx: &egui::Context) {
    TopBottomPanel::bottom("timeline_panel")
        .resizable(true)
        .min_height(200.0)
        .default_height(340.0)
        .show(ctx, |ui| {
            panel_header(ui, "TIMELINE");
            ui.horizontal(|ui| {
                if ui.small_button("+ Video").on_hover_text("Add a video track").clicked() {
                    app.push_undo("track");
                    app.project.add_track(TrackKind::Video);
                    app.mark_dirty();
                }
                if ui.small_button("+ Audio").on_hover_text("Add an audio track").clicked() {
                    app.push_undo("track");
                    app.project.add_track(TrackKind::Audio);
                    app.mark_dirty();
                }
                ui.separator();
                if ui
                    .small_button("Title")
                    .on_hover_text("Add a text/title clip at the playhead")
                    .clicked()
                {
                    app.add_title_action();
                }
                ui.separator();
                if ui
                    .toggle_value(&mut app.tl.snap, "🧲")
                    .on_hover_text("Magnet snapping to clip edges and playhead")
                    .changed()
                {
                    // nothing else needed
                }
                if ui.small_button("Split").on_hover_text("S — cut at the playhead").clicked() {
                    app.split_action();
                }
                if ui
                    .small_button("Delete")
                    .on_hover_text("Del — remove selected clip")
                    .clicked()
                {
                    app.delete_selected();
                }
                if ui
                    .small_button("Ripple")
                    .on_hover_text("Shift+Del — remove selected clip and close the gap")
                    .clicked()
                {
                    app.ripple_delete_selected();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let zoom = Slider::new(&mut app.tl.px_per_sec, 4.0..=400.0)
                        .logarithmic(true);
                    ui.add_sized([120.0, 18.0], zoom).on_hover_text("Zoom");
                });
            });
            ui.add_space(4.0);

            let n_tracks = app.project.tracks.len().max(1);
            let lane_h = ((ui.available_height() - RULER_H - 10.0) / n_tracks as f32)
                .clamp(MIN_LANE_H, MAX_LANE_H);
            let total_h = RULER_H + lane_h * app.project.tracks.len() as f32;

            ui.horizontal(|ui| {
                // Allocate BOTH rects up-front: the header slots and canvas
                // share the same total height so their tops always align.
                // (Drawing must happen after both allocations — ui.put()
                // advances the row cursor, so mixing draws and allocations
                // would shift/overlap the canvas.)
                let (col, _) = ui.allocate_exact_size(Vec2::new(HEADER_W, total_h), Sense::hover());
                ui.add_space(4.0);
                let canvas_w = (ui.available_width() - 8.0).max(120.0);
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(canvas_w, total_h), Sense::click_and_drag());
                draw_headers(app, ui, col, total_h, lane_h);
                canvas(app, ui, rect, resp, lane_h, canvas_w);
            });
        });
}

// ---------------------------------------------------------------------------

/// Track header column: one slot per lane (exact same height/order as the
/// canvas lanes), each with the track name and Mute / Solo / Lock / Delete.
/// Painted into the pre-allocated `col` rect (no layout of its own).
fn draw_headers(app: &mut VeditApp, ui: &mut egui::Ui, col: Rect, total_h: f32, lane_h: f32) {
    let _ = total_h;
    // Owned painter (cloned) so it can be used between ui.put() calls.
    let p = ui.painter().clone();
    p.rect_filled(col, 0.0, Palette::PANEL);

    // Ruler corner (aligns with the canvas ruler row).
    let ruler = Rect::from_min_max(col.min, Pos2::new(col.max.x, col.min.y + RULER_H));
    p.text(
        Pos2::new(ruler.left() + 6.0, ruler.top() + 7.0),
        Align2::LEFT_TOP,
        "TRACKS",
        FontId::proportional(8.5),
        Palette::TEXT_DIM,
    );
    p.line_segment(
        [Pos2::new(ruler.left(), ruler.bottom()), Pos2::new(ruler.right(), ruler.bottom())],
        Stroke::new(1.0_f32, Palette::LINE),
    );

    // Snapshot track data so we can mutate the project inside the loop.
    let tracks: Vec<(u64, String, TrackKind, bool, bool, bool)> = app
        .project
        .tracks
        .iter()
        .map(|t| (t.id, t.name.clone(), t.kind, t.muted, t.solo, t.locked))
        .collect();

    for (i, (id, name, kind, muted, solo, locked)) in tracks.into_iter().enumerate() {
        let slot = Rect::from_min_max(
            Pos2::new(col.left(), col.top() + RULER_H + i as f32 * lane_h),
            Pos2::new(col.right(), col.top() + RULER_H + (i + 1) as f32 * lane_h),
        );
        p.rect_filled(slot, 0.0, Palette::PANEL);
        p.line_segment(
            [Pos2::new(slot.left(), slot.bottom()), Pos2::new(slot.right(), slot.bottom())],
            Stroke::new(1.0_f32, Palette::LINE),
        );

        // Kind dot + track name, vertically centered on the left.
        let dot = match kind {
            TrackKind::Video => Palette::CLIP_VIDEO_BAR,
            TrackKind::Audio => Palette::CLIP_AUDIO_BAR,
        };
        p.circle_filled(Pos2::new(slot.left() + 8.0, slot.center().y), 2.5, dot);
        p.text(
            Pos2::new(slot.left() + 15.0, slot.center().y),
            Align2::LEFT_CENTER,
            name,
            FontId::proportional(11.0),
            if muted { Palette::TEXT_DIM } else { Palette::TEXT },
        );

        // Settings buttons: [M] [S] [L] [×], right-aligned, vertically centered.
        let bsz = Vec2::new(18.0, 16.0);
        let by = slot.center().y - bsz.y * 0.5;
        let bx = slot.right() - 5.0;

        let del_rect = Rect::from_min_size(Pos2::new(bx - bsz.x, by), bsz);
        let del_btn = ui.put(
            del_rect,
            Button::new(RichText::new("×").size(9.5).color(Palette::TEXT_DIM))
                .fill(Palette::SURFACE)
                .small(),
        );
        if del_btn.on_hover_text("Delete track").clicked() {
            let n_clips = app.project.track(id).map(|t| t.clips.len()).unwrap_or(0);
            app.push_undo("track");
            if app.project.remove_track(id) {
                if app.selected.map(|(t, _)| t == id).unwrap_or(false) {
                    app.selected = None;
                }
                app.mark_dirty();
                app.notify(format!("Track removed ({n_clips} clips)"));
                app.request_preview_frame();
            } else {
                app.history.undo(&app.project);
            }
        }

        let lock_rect = Rect::from_min_size(Pos2::new(bx - 2.0 * bsz.x - 2.0, by), bsz);
        let lock_btn = ui.put(
            lock_rect,
            Button::new(
                RichText::new("L")
                    .size(9.0)
                    .color(if locked { Palette::ACCENT } else { Palette::TEXT_DIM }),
            )
            .fill(if locked {
                Color32::from_rgb(48, 40, 24)
            } else {
                Palette::SURFACE
            })
            .small(),
        );
        if lock_btn.on_hover_text("Lock track (blocks edits)").clicked() {
            if let Some(t) = app.project.track_mut(id) {
                t.locked = !t.locked;
            }
            app.mark_dirty();
        }

        let solo_rect = Rect::from_min_size(Pos2::new(bx - 3.0 * bsz.x - 4.0, by), bsz);
        let solo_btn = ui.put(
            solo_rect,
            Button::new(
                RichText::new("S")
                    .size(9.0)
                    .color(if solo { Palette::ACCENT } else { Palette::TEXT_DIM }),
            )
            .fill(if solo {
                Color32::from_rgb(48, 40, 24)
            } else {
                Palette::SURFACE
            })
            .small(),
        );
        if solo_btn.on_hover_text("Solo track (mutes the others on export)").clicked() {
            app.push_undo("mix");
            if let Some(t) = app.project.track_mut(id) {
                t.solo = !t.solo;
            }
            app.mark_dirty();
        }

        let mute_rect = Rect::from_min_size(Pos2::new(bx - 4.0 * bsz.x - 6.0, by), bsz);
        let mute_btn = ui.put(
            mute_rect,
            Button::new(
                RichText::new("M")
                    .size(9.0)
                    .color(if muted { Palette::DANGER } else { Palette::TEXT_DIM }),
            )
            .fill(if muted {
                Color32::from_rgb(45, 28, 26)
            } else {
                Palette::SURFACE
            })
            .small(),
        );
        if mute_btn.on_hover_text("Mute track").clicked() {
            app.push_undo("mix");
            if let Some(t) = app.project.track_mut(id) {
                t.muted = !t.muted;
            }
            app.mark_dirty();
        }
    }
}

// ---------------------------------------------------------------------------

fn canvas(
    app: &mut VeditApp,
    ui: &mut egui::Ui,
    rect: Rect,
    resp: egui::Response,
    lane_h: f32,
    canvas_w: f32,
) {
    let pps = app.tl.px_per_sec.max(1.0);
    let scroll_x = app.tl.scroll_x.max(0.0);
    let lanes_top = rect.top() + RULER_H;
    let n = app.project.tracks.len();
    let lanes_rect = Rect::from_min_max(
        Pos2::new(rect.left(), lanes_top),
        Pos2::new(rect.right(), lanes_top + lane_h * n.max(1) as f32),
    );
    let time_at = |x: f32| (((x - rect.left() + scroll_x) / pps) as f64).max(0.0);
    let x_at = |t: f64| rect.left() + (t as f32 * pps) - scroll_x;

    // Build draw list (owned data → no borrow conflicts later).
    let selected = app.selected;
    let mut clip_draws: Vec<ClipDraw> = Vec::new();
    for (i, t) in app.project.tracks.iter().enumerate() {
        for (idx, c) in t.clips.iter().enumerate() {
            let (fill, bar) = if c.is_text() {
                (Palette::CLIP_TITLE, Palette::CLIP_TITLE_BAR)
            } else {
                app_clip_colors(app, c.asset_id)
            };
            let label = if c.is_text() {
                c.text
                    .as_ref()
                    .map(|tx| tx.content.lines().next().unwrap_or("Title").to_string())
                    .unwrap_or_else(|| "Title".into())
            } else {
                asset_label(app, c.asset_id)
            };
            // Effective cross-dissolve window against the NEXT clip.
            let transition_d = timeline::effective_overlap(&app.project, t.id, c);
            let _ = idx;
            clip_draws.push(ClipDraw {
                track_idx: i,
                track_id: t.id,
                id: c.id,
                start: c.start,
                dur: c.duration,
                speed: c.speed,
                in_point: c.in_point,
                asset_id: c.asset_id,
                kind: app.project.asset(c.asset_id).map(|a| a.kind),
                is_text: c.is_text(),
                text_label: label.clone(),
                fade_in: c.fade_in,
                fade_out: c.fade_out,
                transition_d,
                muted: t.muted || c.muted,
                locked: t.locked,
                selected: selected == Some((t.id, c.id)),
                fill,
                bar,
                label,
            });
        }
    }

    let painter = ui.painter_at(rect);

    // Backgrounds
    painter.rect_filled(
        Rect::from_min_max(rect.left_top(), Pos2::new(rect.right(), lanes_top)),
        0.0,
        Palette::PANEL,
    );
    painter.rect_filled(lanes_rect, 0.0, Palette::BG);
    for i in 0..n {
        if i % 2 == 1 {
            let row = Rect::from_min_max(
                Pos2::new(rect.left(), lanes_top + i as f32 * lane_h),
                Pos2::new(rect.right(), lanes_top + (i + 1) as f32 * lane_h),
            );
            painter.rect_filled(row, 0.0, Color32::from_rgb(24, 24, 27));
        }
    }

    // Ruler ticks + lane grid
    let step = nice_step(pps);
    let t0 = time_at(rect.left());
    let t1 = time_at(rect.right());
    let mut t = (t0 / step).floor() * step;
    while t <= t1 + step {
        let x = x_at(t);
        painter.line_segment(
            [Pos2::new(x, rect.top() + RULER_H - 9.0), Pos2::new(x, lanes_top)],
            Stroke::new(1.0_f32, Palette::LINE),
        );
        painter.text(
            Pos2::new(x + 4.0, rect.top() + 4.0),
            Align2::LEFT_TOP,
            fmt_ruler_label(t, step),
            FontId::monospace(9.5),
            Palette::TEXT_DIM,
        );
        for k in 1..4 {
            let tm = t + step * k as f64 / 4.0;
            let xm = x_at(tm);
            painter.line_segment(
                [Pos2::new(xm, rect.top() + RULER_H - 5.0), Pos2::new(xm, lanes_top)],
                Stroke::new(1.0_f32, Color32::from_rgb(34, 34, 38)),
            );
        }
        painter.line_segment(
            [Pos2::new(x, lanes_top), Pos2::new(x, lanes_rect.bottom())],
            Stroke::new(1.0_f32, Color32::from_rgb(23, 23, 26)),
        );
        t += step;
    }

    // Clips
    for cd in &clip_draws {
        let x0 = x_at(cd.start);
        let x1 = x_at(cd.start + cd.dur);
        if x1 < rect.left() - 2.0 || x0 > rect.right() + 2.0 {
            continue;
        }
        let y0 = lanes_top + cd.track_idx as f32 * lane_h + 2.0;
        let y1 = y0 + lane_h - 4.0;
        let crect = Rect::from_min_max(
            Pos2::new(x0.max(rect.left()), y0),
            Pos2::new(x1.min(rect.right()), y1),
        );
        if crect.width() < 1.0 {
            continue;
        }
        let cp = ui.painter_at(crect);
        let mut fill = if cd.muted { dim_color(cd.fill) } else { cd.fill };
        if cd.locked {
            fill = dim_color(fill);
        }
        cp.rect_filled(crect, egui::Rounding::same(3.0), fill);
        cp.rect_filled(
            Rect::from_min_max(crect.min, Pos2::new(crect.min.x + 3.0, crect.max.y)),
            egui::Rounding::same(2.0),
            cd.bar,
        );

        // ---- content graphics (v0.2.1) ----
        if !cd.muted && !cd.locked {
            match cd.kind {
                Some(MediaKind::Audio) => {
                    if let Some(peaks) = app.waveforms.get(&cd.asset_id) {
                        let asset_dur = app
                            .project
                            .asset(cd.asset_id)
                            .and_then(|a| a.duration())
                            .unwrap_or(cd.dur * cd.speed);
                        draw_waveform(&cp, crect, peaks, asset_dur, cd, pps);
                    }
                }
                Some(MediaKind::Video) | Some(MediaKind::Image) => {
                    if let Some(strip) = app.strips.get(&cd.asset_id) {
                        let asset_dur = app
                            .project
                            .asset(cd.asset_id)
                            .and_then(|a| a.duration())
                            .unwrap_or(cd.dur * cd.speed);
                        draw_filmstrip(&cp, crect, strip, asset_dur, cd, pps);
                    }
                }
                None => {}
            }
        }
        if cd.is_text && crect.width() > 28.0 {
            // Title badge + first text line.
            let badge = Rect::from_min_size(
                Pos2::new(crect.left() + 6.0, crect.center().y - 8.0),
                Vec2::new(16.0, 16.0),
            );
            cp.rect_filled(badge, egui::Rounding::same(3.0), Palette::CLIP_TITLE_BAR);
            cp.text(
                badge.center(),
                Align2::CENTER_CENTER,
                "T",
                FontId::proportional(10.5),
                Palette::ACCENT_TEXT,
            );
            cp.text(
                Pos2::new(crect.left() + 27.0, crect.center().y),
                Align2::LEFT_CENTER,
                cd.text_label.clone(),
                FontId::proportional(10.5),
                Palette::CLIP_TEXT,
            );
        }

        // Fade triangles (video & audio alike).
        if cd.fade_in > 0.01 && !cd.muted {
            let w = (cd.fade_in as f32 * pps).min(crect.width() * 0.9);
            let tri = vec![
                Pos2::new(crect.left(), crect.bottom()),
                Pos2::new(crect.left(), crect.top()),
                Pos2::new(crect.left() + w, crect.top()),
            ];
            cp.add(Shape::convex_polygon(
                tri,
                Color32::from_rgba_unmultiplied(224, 180, 88, 46),
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(224, 180, 88, 110)),
            ));
        }
        if cd.fade_out > 0.01 && !cd.muted {
            let w = (cd.fade_out as f32 * pps).min(crect.width() * 0.9);
            let tri = vec![
                Pos2::new(crect.right(), crect.bottom()),
                Pos2::new(crect.right(), crect.top()),
                Pos2::new(crect.right() - w, crect.top()),
            ];
            cp.add(Shape::convex_polygon(
                tri,
                Color32::from_rgba_unmultiplied(224, 180, 88, 46),
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(224, 180, 88, 110)),
            ));
        }

        // Cross-dissolve marker over the final transition window.
        if cd.transition_d > 0.01 {
            let w = (cd.transition_d as f32 * pps).min(crect.width() * 0.9);
            let zone = Rect::from_min_max(
                Pos2::new(crect.right() - w, crect.top()),
                Pos2::new(crect.right(), crect.bottom()),
            );
            cp.rect_filled(zone, egui::Rounding::same(2.0), Color32::from_rgba_unmultiplied(224, 180, 88, 26));
            cp.line_segment([zone.left_top(), zone.right_bottom()], Stroke::new(1.2_f32, Palette::ACCENT_DIM));
            cp.line_segment([zone.left_bottom(), zone.right_top()], Stroke::new(1.2_f32, Palette::ACCENT_DIM));
            if zone.width() > 26.0 {
                cp.text(
                    zone.center(),
                    Align2::CENTER_CENTER,
                    "xx",
                    FontId::monospace(8.0),
                    Palette::ACCENT,
                );
            }
        }

        if cd.selected {
            cp.rect_stroke(crect, egui::Rounding::same(3.0), Stroke::new(1.5_f32, Palette::ACCENT));
        }
        if crect.width() > 24.0 && !cd.is_text {
            let label = if (cd.speed - 1.0).abs() > 0.01 {
                format!("{}  ×{:.2}", cd.label, cd.speed)
            } else {
                cd.label.clone()
            };
            // Label sits on a subtle dark chip for legibility over filmstrips.
            let galley_bg = Rect::from_min_size(
                Pos2::new(crect.min.x + 5.0, crect.min.y + 1.0),
                Vec2::new((label.len() as f32 * 6.2 + 8.0).min(crect.width() - 10.0), 15.0),
            );
            cp.rect_filled(galley_bg, egui::Rounding::same(2.0), Color32::from_rgba_unmultiplied(10, 10, 12, 150));
            cp.text(
                Pos2::new(crect.min.x + 8.0, galley_bg.center().y),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(10.0),
                if cd.muted { Palette::TEXT_DIM } else { Palette::CLIP_TEXT },
            );
        }
    }

    // Drop highlight while dragging an asset from the library
    let ctx = ui.ctx().clone();
    if DragAndDrop::payload::<u64>(&ctx).is_some() {
        if let Some(p) = ui.input(|i| i.pointer.latest_pos()) {
            if lanes_rect.contains(p) {
                let lane = lane_index(p.y, lanes_top, lane_h, n);
                if let Some(i) = lane {
                    let row = Rect::from_min_max(
                        Pos2::new(rect.left(), lanes_top + i as f32 * lane_h),
                        Pos2::new(rect.right(), lanes_top + (i + 1) as f32 * lane_h),
                    );
                    painter.rect_stroke(row, 0.0, Stroke::new(1.2_f32, Palette::ACCENT));
                }
            }
        }
    }

    // Playhead
    let px = x_at(app.playhead);
    if px >= rect.left() - 1.0 && px <= rect.right() + 1.0 {
        painter.line_segment(
            [Pos2::new(px, rect.top()), Pos2::new(px, lanes_rect.bottom())],
            Stroke::new(1.5_f32, Palette::ACCENT),
        );
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(px - 4.0, rect.top() + 2.0), Vec2::new(8.0, RULER_H - 12.0)),
            egui::Rounding::same(2.0),
            Palette::ACCENT,
        );
    }

    // Interactions
    handle_timeline_input(app, ui, &resp, rect, lanes_top, lane_h, n, pps, scroll_x, canvas_w, &clip_draws);

    // Auto-scroll to keep the playhead visible during playback
    if app.playing {
        let phx = app.playhead as f32 * pps - scroll_x;
        if phx > canvas_w - 60.0 || phx < 0.0 {
            app.tl.scroll_x = (app.playhead as f32 * pps - 60.0).max(0.0);
        }
    }
}

/// Green min/max-style waveform bars mapped through source time (so trims
/// and speed show the right section of the audio).
fn draw_waveform(
    p: &egui::Painter,
    crect: Rect,
    peaks: &[f32],
    asset_dur: f64,
    cd: &ClipDraw,
    pps: f32,
) {
    let n = peaks.len();
    if n == 0 || asset_dur <= 0.0 {
        return;
    }
    let center = crect.center().y;
    let half = crect.height() * 0.40;
    let stroke = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(127, 176, 138, 200));
    let cols = crect.width().floor() as i32;
    for col in 0..cols {
        let x = crect.left() + col as f32 + 0.5;
        let local_t = ((x - crect.left()) / pps) as f64;
        let src_t = cd.in_point + local_t * cd.speed;
        let bucket = ((src_t / asset_dur) * n as f64).floor().max(0.0) as usize;
        let amp = peaks.get(bucket.min(n - 1)).copied().unwrap_or(0.0) as f32;
        let h = (amp * half).max(0.5);
        p.line_segment([Pos2::new(x, center - h), Pos2::new(x, center + h)], stroke);
    }
}

/// Tile the asset's filmstrip across the clip, mapped through source time.
fn draw_filmstrip(
    p: &egui::Painter,
    crect: Rect,
    strip: &StripTex,
    asset_dur: f64,
    cd: &ClipDraw,
    pps: f32,
) {
    let tiles = strip.tiles.max(1) as f64;
    if asset_dur <= 0.0 {
        return;
    }
    let src_to_x = |src_t: f64| crect.left() + ((src_t - cd.in_point) / cd.speed) as f32 * pps;
    let k0 = ((cd.in_point / asset_dur) * tiles).floor().max(0.0) as usize;
    let k1 = (((cd.in_point + cd.dur * cd.speed) / asset_dur) * tiles)
        .ceil()
        .min(tiles) as usize;
    let tex_id = strip.tex.id();
    for k in k0..k1.max(k0 + 1) {
        let st0 = asset_dur * k as f64 / tiles;
        let st1 = asset_dur * (k + 1) as f64 / tiles;
        let x0 = src_to_x(st0);
        let x1 = src_to_x(st1);
        if x1 < crect.left() || x0 > crect.right() || x1 <= x0 {
            continue;
        }
        let u0 = k as f32 / tiles as f32;
        let u1 = (k + 1) as f32 / tiles as f32;
        let uv = Rect::from_min_max(Pos2::new(u0, 0.0), Pos2::new(u1, 1.0));
        let dst = Rect::from_min_max(
            Pos2::new(x0.max(crect.left()), crect.top()),
            Pos2::new(x1.min(crect.right()), crect.bottom()),
        );
        p.image(tex_id, dst, uv, Color32::WHITE);
    }
}

fn app_clip_colors(app: &VeditApp, asset_id: u64) -> (Color32, Color32) {
    match app.project.asset(asset_id).map(|a| a.kind) {
        Some(MediaKind::Audio) => (Palette::CLIP_AUDIO, Palette::CLIP_AUDIO_BAR),
        Some(MediaKind::Image) => (Palette::CLIP_IMAGE, Palette::CLIP_IMAGE_BAR),
        Some(MediaKind::Video) => (Palette::CLIP_VIDEO, Palette::CLIP_VIDEO_BAR),
        None => (Color32::from_rgb(60, 60, 60), Palette::TEXT_DIM),
    }
}

fn asset_label(app: &VeditApp, asset_id: u64) -> String {
    app.project
        .asset(asset_id)
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "(missing)".into())
}

// ---------------------------------------------------------------------------

fn handle_timeline_input(
    app: &mut VeditApp,
    ui: &egui::Ui,
    resp: &egui::Response,
    rect: Rect,
    lanes_top: f32,
    lane_h: f32,
    n_tracks: usize,
    pps: f32,
    scroll_x: f32,
    _canvas_w: f32,
    clips: &[ClipDraw],
) {
    // Wheel: horizontal scroll / Ctrl+Wheel: zoom anchored at pointer
    let (scroll_delta, ctrl) = ui.input(|i| (i.raw_scroll_delta, i.modifiers.ctrl));
    if scroll_delta != Vec2::ZERO && resp.hovered() {
        if ctrl {
            if let Some(p) = resp.hover_pos() {
                let t = ((p.x - rect.left() + scroll_x) / pps) as f64;
                let factor = 1.0 + (-scroll_delta.y) * 0.0015;
                let new_pps = (app.tl.px_per_sec * factor).clamp(4.0, 400.0);
                app.tl.scroll_x = (t as f32 * new_pps - (p.x - rect.left())).max(0.0);
                app.tl.px_per_sec = new_pps;
            }
        } else {
            app.tl.scroll_x = (app.tl.scroll_x - scroll_delta.x - scroll_delta.y * 0.6).max(0.0);
        }
    }

    let dur = app.project.duration();
    let pos = resp.interact_pointer_pos().or_else(|| resp.hover_pos());
    let time_at = |x: f32| (((x - rect.left() + scroll_x) / pps) as f64).max(0.0);

    // Start drag / click
    if resp.drag_started() {
        if let Some(p) = pos {
            let t = time_at(p.x);
            if p.y < lanes_top {
                app.tl.drag = Some(DragState::Scrub);
                app.seek_to(t.min(dur));
            } else if let Some(hit) = hit_test(clips, lanes_top, lane_h, n_tracks, p, pps, scroll_x, rect) {
                app.selected = Some((hit.track_id, hit.clip_id));
                let locked = app.project.track(hit.track_id).map(|t| t.locked).unwrap_or(false);
                app.tl.drag = match hit.edge {
                    Edge::Start if !locked => {
                        app.push_undo("trim");
                        Some(DragState::TrimStart {
                            track_id: hit.track_id,
                            clip_id: hit.clip_id,
                        })
                    }
                    Edge::End if !locked => {
                        app.push_undo("trim");
                        Some(DragState::TrimEnd {
                            track_id: hit.track_id,
                            clip_id: hit.clip_id,
                        })
                    }
                    Edge::None if !locked => {
                        let start = app
                            .project
                            .find_clip(hit.track_id, hit.clip_id)
                            .map(|c| c.start)
                            .unwrap_or(0.0);
                        app.push_undo("move");
                        Some(DragState::Move {
                            track_id: hit.track_id,
                            clip_id: hit.clip_id,
                            grab_offset: t - start,
                        })
                    }
                    _ => None,
                };
            } else {
                app.selected = None;
                app.tl.drag = Some(DragState::Scrub);
                app.seek_to(t.min(dur));
            }
        }
    }

    // Drop from media library (payload survives until the release frame)
    let released = ui.input(|i| i.pointer.any_released());
    if released {
        if let Some(asset_id) = DragAndDrop::take_payload::<u64>(ui.ctx()) {
            if let Some(p) = pos {
                if p.y >= lanes_top {
                    let t = time_at(p.x);
                    let lane = lane_index(p.y, lanes_top, lane_h, n_tracks);
                    drop_asset(app, *asset_id, lane, t);
                }
            }
        }
    }

    // Drag updates
    if resp.dragged() {
        if let Some(p) = pos {
            let t = time_at(p.x);
            match app.tl.drag.clone() {
                Some(DragState::Scrub) => {
                    app.seek_to(t.min(dur));
                }
                Some(DragState::Move { track_id, clip_id, grab_offset }) => {
                    let dur_c = app
                        .project
                        .find_clip(track_id, clip_id)
                        .map(|c| c.duration)
                        .unwrap_or(0.0);
                    let raw = (t - grab_offset).max(0.0);
                    let start = if app.tl.snap {
                        let cands = snap_candidates(app, track_id, clip_id);
                        let tol = (SNAP_TOL_PX / pps) as f64;
                        snap_pair(&cands, raw, raw + dur_c, tol)
                    } else {
                        raw
                    };
                    let target = lane_track_id(app, p.y, lanes_top, lane_h, n_tracks, track_id)
                        .unwrap_or(track_id);
                    if timeline::move_clip(&mut app.project, track_id, clip_id, target, start) {
                        app.selected = Some((target, clip_id));
                        app.tl.drag = Some(DragState::Move {
                            track_id: target,
                            clip_id,
                            grab_offset,
                        });
                    }
                    app.mark_dirty();
                }
                Some(DragState::TrimStart { track_id, clip_id }) => {
                    let s = if app.tl.snap {
                        let cands = snap_candidates(app, track_id, clip_id);
                        let tol = (SNAP_TOL_PX / pps) as f64;
                        snap_time(&cands, t, tol).unwrap_or(t)
                    } else {
                        t
                    };
                    timeline::trim_start(&mut app.project, track_id, clip_id, s);
                    app.mark_dirty();
                }
                Some(DragState::TrimEnd { track_id, clip_id }) => {
                    let e = if app.tl.snap {
                        let cands = snap_candidates(app, track_id, clip_id);
                        let tol = (SNAP_TOL_PX / pps) as f64;
                        snap_time(&cands, t, tol).unwrap_or(t)
                    } else {
                        t
                    };
                    timeline::trim_end(&mut app.project, track_id, clip_id, e);
                    app.mark_dirty();
                }
                None => {}
            }
        }
    }

    // End drag
    if resp.drag_stopped() {
        let was_editing = matches!(
            app.tl.drag,
            Some(DragState::Move { .. }) | Some(DragState::TrimStart { .. }) | Some(DragState::TrimEnd { .. })
        );
        app.tl.drag = None;
        if was_editing {
            app.request_preview_frame();
        }
    }

    // Plain click on the ruler seeks.
    if resp.clicked() {
        if let Some(p) = pos {
            if p.y < lanes_top {
                app.seek_to(time_at(p.x).min(dur));
            }
        }
    }

    // Hover cursors
    if !resp.dragged() {
        if let Some(p) = resp.hover_pos() {
            let icon = if p.y >= lanes_top {
                match hit_test(clips, lanes_top, lane_h, n_tracks, p, pps, scroll_x, rect) {
                    Some(hit) => match hit.edge {
                        Edge::Start => CursorIcon::ResizeWest,
                        Edge::End => CursorIcon::ResizeEast,
                        Edge::None => CursorIcon::Grab,
                    },
                    None => CursorIcon::Crosshair,
                }
            } else {
                CursorIcon::Crosshair
            };
            resp.clone().on_hover_cursor(icon);
        }
    }
}

fn drop_asset(app: &mut VeditApp, asset_id: u64, lane: Option<usize>, t: f64) {
    let Some(asset) = app.project.asset(asset_id).cloned() else {
        return;
    };
    let kind = match asset.kind {
        MediaKind::Audio => TrackKind::Audio,
        _ => TrackKind::Video,
    };
    let track_id = lane
        .and_then(|i| app.project.tracks.get(i))
        .filter(|t| t.kind == kind && !t.locked)
        .map(|t| t.id)
        .or_else(|| timeline::find_target_track(&app.project, kind, None));
    let Some(tid) = track_id else {
        app.notify(format!("No unlocked {} track available", kind.label()));
        return;
    };
    app.push_undo("drop");
    match timeline::add_asset_clip_auto(&mut app.project, asset_id, tid, t) {
        Ok(made) => {
            let (vtid, vcid) = made[0];
            let name = app.project.track(vtid).map(|t| t.name.clone()).unwrap_or_default();
            app.selected = Some((vtid, vcid));
            app.mark_dirty();
            if made.len() > 1 {
                app.notify(format!("Added {} to {name} (linked A/V)", asset.name));
            } else {
                app.notify(format!("Added {} to {name}", asset.name));
            }
            app.request_preview_frame();
        }
        Err(e) => {
            app.history.undo(&app.project);
            app.notify(format!("Could not add clip: {e}"));
        }
    }
}

fn lane_index(y: f32, lanes_top: f32, lane_h: f32, n: usize) -> Option<usize> {
    if y < lanes_top {
        return None;
    }
    let idx = ((y - lanes_top) / lane_h) as usize;
    if idx < n {
        Some(idx)
    } else {
        None
    }
}

fn lane_track_id(
    app: &VeditApp,
    y: f32,
    lanes_top: f32,
    lane_h: f32,
    n: usize,
    source_track: u64,
) -> Option<u64> {
    let idx = lane_index(y, lanes_top, lane_h, n)?;
    let target = app.project.tracks.get(idx)?;
    let source_kind = app.project.track(source_track).map(|t| t.kind)?;
    if target.kind == source_kind && !target.locked {
        Some(target.id)
    } else {
        None
    }
}

fn hit_test(
    clips: &[ClipDraw],
    lanes_top: f32,
    lane_h: f32,
    n: usize,
    p: Pos2,
    pps: f32,
    scroll_x: f32,
    rect: Rect,
) -> Option<Hit> {
    let lane = lane_index(p.y, lanes_top, lane_h, n)?;
    let t = ((p.x - rect.left() + scroll_x) / pps) as f64;
    for cd in clips.iter().filter(|c| c.track_idx == lane) {
        if t >= cd.start && t <= cd.start + cd.dur {
            let x0 = rect.left() + (cd.start as f32 * pps) - scroll_x;
            let x1 = rect.left() + ((cd.start + cd.dur) as f32 * pps) - scroll_x;
            let wide = x1 - x0 > 16.0;
            let edge = if wide && p.x <= x0 + EDGE_PX {
                Edge::Start
            } else if wide && p.x >= x1 - EDGE_PX {
                Edge::End
            } else {
                Edge::None
            };
            return Some(Hit {
                track_id: cd.track_id,
                clip_id: cd.id,
                edge,
            });
        }
    }
    None
}

fn snap_candidates(app: &VeditApp, ex_track: u64, ex_clip: u64) -> Vec<f64> {
    let mut v = vec![0.0, app.playhead];
    for t in &app.project.tracks {
        for c in &t.clips {
            if t.id == ex_track && c.id == ex_clip {
                continue;
            }
            v.push(c.start);
            v.push(c.end());
        }
    }
    v
}

fn snap_pair(cands: &[f64], a: f64, b: f64, tol: f64) -> f64 {
    match (snap_time(cands, a, tol), snap_time(cands, b, tol)) {
        (Some(sa), Some(sb)) => {
            if (sa - a).abs() <= (sb - b).abs() {
                sa
            } else {
                sb - (b - a)
            }
        }
        (Some(sa), None) => sa,
        (None, Some(sb)) => sb - (b - a),
        (None, None) => a,
    }
}

fn nice_step(pps: f32) -> f64 {
    for step in [
        0.05f64, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0,
    ] {
        if step * (pps as f64) >= 70.0 {
            return step;
        }
    }
    600.0
}

fn fmt_ruler_label(t: f64, step: f64) -> String {
    if step < 1.0 {
        return format!("{t:.2}s");
    }
    let total = t.floor() as u32;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

fn dim_color(c: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (c.r() as f32 * 0.45) as u8,
        (c.g() as f32 * 0.45) as u8,
        (c.b() as f32 * 0.45) as u8,
        255,
    )
}
