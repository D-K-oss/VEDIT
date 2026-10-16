//! Timeline panel: ruler, tracks, clips, playhead, drag/trim/snap, drops.

use eframe::egui::{
    self, Align2, Button, Color32, CursorIcon, DragAndDrop, FontId, Pos2, Rect, RichText, Sense,
    Slider, Stroke, TopBottomPanel, Vec2,
};

use vedit_core::media::MediaKind;
use vedit_core::project::TrackKind;
use vedit_core::timeline::{self, snap_time};

use crate::app::VeditApp;
use crate::theme::Palette;

const HEADER_W: f32 = 96.0;
const RULER_H: f32 = 26.0;
const MIN_LANE_H: f32 = 22.0;
const MAX_LANE_H: f32 = 46.0;
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
    muted: bool,
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
        .min_height(210.0)
        .default_height(330.0)
        .show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.small_button("+ Video track").clicked() {
                    app.project.add_track(TrackKind::Video);
                    app.mark_dirty();
                }
                if ui.small_button("+ Audio track").clicked() {
                    app.project.add_track(TrackKind::Audio);
                    app.mark_dirty();
                }
                ui.separator();
                if ui.toggle_value(&mut app.tl.snap, "Snap").changed() {
                    // nothing else needed
                }
                ui.separator();
                if ui.small_button("Split").on_hover_text("S").clicked() {
                    app.split_action();
                }
                if ui.small_button("Delete").on_hover_text("Del").clicked() {
                    app.delete_selected();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(
                        Slider::new(&mut app.tl.px_per_sec, 4.0..=400.0)
                            .logarithmic(true)
                            .text("Zoom"),
                    );
                });
            });
            ui.add_space(4.0);

            let n_tracks = app.project.tracks.len().max(1);
            let lane_h = ((ui.available_height() - RULER_H - 10.0) / n_tracks as f32)
                .clamp(MIN_LANE_H, MAX_LANE_H);
            let canvas_w = (ui.available_width() - HEADER_W - 12.0).max(120.0);

            ui.horizontal(|ui| {
                headers_column(app, ui, lane_h);
                ui.add_space(4.0);
                let total_h = RULER_H + lane_h * app.project.tracks.len() as f32;
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(canvas_w, total_h), Sense::click_and_drag());
                canvas(app, ui, rect, resp, lane_h, canvas_w);
            });
        });
}

// ---------------------------------------------------------------------------

fn headers_column(app: &mut VeditApp, ui: &mut egui::Ui, lane_h: f32) {
    // Ruler spacer
    let (ruler_rect, _) = ui.allocate_exact_size(Vec2::new(HEADER_W, RULER_H), Sense::hover());
    ui.painter().text(
        Pos2::new(ruler_rect.left() + 6.0, ruler_rect.top() + 7.0),
        Align2::LEFT_TOP,
        "TIMECODE",
        FontId::proportional(8.5),
        Palette::TEXT_DIM,
    );

    let headers: Vec<(u64, String, TrackKind, bool)> = app
        .project
        .tracks
        .iter()
        .map(|t| (t.id, t.name.clone(), t.kind, t.muted))
        .collect();

    for (id, name, kind, muted) in headers {
        let (r, _) = ui.allocate_exact_size(Vec2::new(HEADER_W, lane_h), Sense::hover());
        let p = ui.painter();
        p.rect_filled(r, 0.0, Palette::PANEL);
        p.line_segment(
            [Pos2::new(r.left(), r.bottom()), Pos2::new(r.right(), r.bottom())],
            Stroke::new(1.0_f32, Palette::LINE),
        );
        p.text(
            Pos2::new(r.left() + 8.0, r.top() + 4.0),
            Align2::LEFT_TOP,
            name,
            FontId::proportional(11.0),
            if muted { Palette::TEXT_DIM } else { Palette::TEXT },
        );
        let dot = match kind {
            TrackKind::Video => Palette::CLIP_VIDEO_BAR,
            TrackKind::Audio => Palette::CLIP_AUDIO_BAR,
        };
        p.circle_filled(Pos2::new(r.right() - 10.0, r.top() + 10.0), 3.0, dot);

        let bsz = Vec2::new(20.0, 16.0);
        let by = r.bottom() - bsz.y - 4.0;
        let mute_rect = Rect::from_min_size(Pos2::new(r.right() - bsz.x - 5.0, by), bsz);
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
            if let Some(t) = app.project.track_mut(id) {
                t.muted = !t.muted;
            }
            app.mark_dirty();
        }

        let del_rect = Rect::from_min_size(Pos2::new(r.right() - 2.0 * bsz.x - 8.0, by), bsz);
        let del_btn = ui.put(
            del_rect,
            Button::new(RichText::new("×").size(9.0).color(Palette::TEXT_DIM))
                .fill(Palette::SURFACE)
                .small(),
        );
        if del_btn.on_hover_text("Delete track").clicked() {
            let n_clips = app.project.track(id).map(|t| t.clips.len()).unwrap_or(0);
            if app.project.remove_track(id) {
                if app.selected.map(|(t, _)| t == id).unwrap_or(false) {
                    app.selected = None;
                }
                app.mark_dirty();
                app.notify(format!("Track removed ({n_clips} clips)"));
                app.request_preview_frame();
            }
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
        for c in &t.clips {
            let (fill, bar) = app_clip_colors(app, c.asset_id);
            let label = asset_label(app, c.asset_id);
            clip_draws.push(ClipDraw {
                track_idx: i,
                track_id: t.id,
                id: c.id,
                start: c.start,
                dur: c.duration,
                speed: c.speed,
                muted: t.muted || c.muted,
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
        let fill = if cd.muted { dim_color(cd.fill) } else { cd.fill };
        cp.rect_filled(crect, egui::Rounding::same(3.0), fill);
        cp.rect_filled(
            Rect::from_min_max(crect.min, Pos2::new(crect.min.x + 3.0, crect.max.y)),
            egui::Rounding::same(2.0),
            cd.bar,
        );
        if cd.selected {
            cp.rect_stroke(crect, egui::Rounding::same(3.0), Stroke::new(1.5_f32, Palette::ACCENT));
        }
        if crect.width() > 24.0 {
            let label = if (cd.speed - 1.0).abs() > 0.01 {
                format!("{}  ×{:.2}", cd.label, cd.speed)
            } else {
                cd.label.clone()
            };
            cp.text(
                Pos2::new(crect.min.x + 8.0, crect.center().y),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(10.5),
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
                app.tl.drag = match hit.edge {
                    Edge::Start => Some(DragState::TrimStart {
                        track_id: hit.track_id,
                        clip_id: hit.clip_id,
                    }),
                    Edge::End => Some(DragState::TrimEnd {
                        track_id: hit.track_id,
                        clip_id: hit.clip_id,
                    }),
                    Edge::None => {
                        let start = app
                            .project
                            .find_clip(hit.track_id, hit.clip_id)
                            .map(|c| c.start)
                            .unwrap_or(0.0);
                        Some(DragState::Move {
                            track_id: hit.track_id,
                            clip_id: hit.clip_id,
                            grab_offset: t - start,
                        })
                    }
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
    match timeline::add_asset_clip(&mut app.project, asset_id, tid, t) {
        Ok(cid) => {
            let name = app.project.track(tid).map(|t| t.name.clone()).unwrap_or_default();
            app.selected = Some((tid, cid));
            app.mark_dirty();
            app.notify(format!("Added {} to {name}", asset.name));
            app.request_preview_frame();
        }
        Err(e) => app.notify(format!("Could not add clip: {e}")),
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
