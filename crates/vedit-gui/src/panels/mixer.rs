//! Audio workspace: a full Fairlight-style console page — channel strips per
//! audio track (EQ / Dynamics / normalize / pan / S·M·R / fader / meter /
//! FX-order badge) and a MASTER strip with BS.1770-4 loudness readouts.
//! Live in preview, applied on export.

use eframe::egui::{self, Color32, RichText, ScrollArea, SidePanel};

use vedit_core::media::format_timecode;
use vedit_core::project::TrackKind;

use crate::app::VeditApp;
use crate::panels::widgets::{fader, gain_to_db, knob, meter, strip_button};
use crate::theme::{panel_header, Palette};

const SOLO_COLOR: Color32 = Color32::from_rgb(48, 40, 24);
const MUTE_COLOR: Color32 = Color32::from_rgb(45, 28, 26);
const ARM_COLOR: Color32 = Color32::from_rgb(74, 26, 22);

/// The Audio workspace page: library/index sidebar + channel strips.
pub(crate) fn mixer_page(app: &mut VeditApp, ctx: &egui::Context) {
    // Decay meters every frame so bars fall even when audio goes idle.
    app.decay_meters();

    // Left-most sidebar: Fairlight FX library + Index (Markers/Tracks).
    fairlight_sidebar(app, ctx);

    SidePanel::left("mixer_page")
        .resizable(true)
        .default_width(460.0)
        .min_width(300.0)
        .width_range(300.0..=760.0)
        .show(ctx, |ui| {
            panel_header(ui, "FAIRLIGHT  ·  live in preview · applied on export");

            ScrollArea::both().show(ui, |ui| {
                ui.horizontal(|ui| {
                    let tracks: Vec<(usize, u64, String, bool, bool)> = app
                        .project
                        .tracks
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| t.kind == TrackKind::Audio)
                        .map(|(i, t)| (i, t.id, t.name.clone(), t.muted, t.solo))
                        .collect();

                    if tracks.is_empty() {
                        ui.label(
                            RichText::new("No audio tracks yet — add one on the timeline.")
                                .color(Palette::TEXT_FAINT),
                        );
                    }

                    for (index, id, name, muted, solo) in tracks {
                        strip(app, ui, index, id, &name, muted, solo);
                        ui.separator();
                    }

                    master_strip(app, ui);
                });
            });

            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Double-click a fader or knob to reset. Normalize matches the loudest clip to −1 dBFS.",
                )
                .small()
                .color(Palette::TEXT_FAINT),
            );
        });
}

/// One channel strip.
fn strip(app: &mut VeditApp, ui: &mut egui::Ui, track_index: usize, track_id: u64, name: &str, muted: bool, solo: bool) {
    let width = 112.0;
    let fader_h = 190.0;
    egui::Frame::none()
        .fill(Palette::PANEL)
        .stroke(egui::Stroke::new(1.0_f32, Palette::LINE))
        .inner_margin(8.0)
        .rounding(6.0)
        .show(ui, |ui| {
            ui.set_width(width);
            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {

            // Name + kind dot.
            ui.horizontal(|ui| {
                let dot_x = ui.cursor().left() + 2.0;
                let dot_y = ui.cursor().center().y;
                ui.painter().circle_filled(egui::Pos2::new(dot_x, dot_y), 3.0, Palette::CLIP_AUDIO_BAR);
                ui.add_space(8.0);
                ui.label(RichText::new(name).strong().size(12.0));
            });

            // EQ / Dynamics / Normalize inserts + FX order badge.
            ui.horizontal(|ui| {
                let eq_active = app
                    .project
                    .track(track_id)
                    .map(|t| t.eq.enabled && !t.eq.is_neutral())
                    .unwrap_or(false);
                let dyn_active = app
                    .project
                    .track(track_id)
                    .map(|t| t.dynamics.enabled)
                    .unwrap_or(false);
                let comp_active = app
                    .project
                    .track(track_id)
                    .map(|t| t.comp.enabled)
                    .unwrap_or(false);
                if strip_button(ui, "EQ", eq_active, "6-band parametric equalizer", SOLO_COLOR).clicked() {
                    app.eq_dialog_track = if app.eq_dialog_track == Some(track_id) { None } else { Some(track_id) };
                }
                if strip_button(ui, "DYN", dyn_active, "Dynamics: gate → compressor → limiter", SOLO_COLOR).clicked() {
                    app.dyn_dialog_track = if app.dyn_dialog_track == Some(track_id) { None } else { Some(track_id) };
                }
                if strip_button(ui, "Comp", comp_active, "3-band multiband compressor", SOLO_COLOR).clicked() {
                    app.comp_dialog_track = if app.comp_dialog_track == Some(track_id) { None } else { Some(track_id) };
                }
            });
            ui.horizontal(|ui| {
                if strip_button(ui, "Norm", false, "Normalize: loudest clip on this track → −1 dBFS", SOLO_COLOR).clicked() {
                    app.normalize_track(track_id);
                }
                // FX order badge (Fairlight shows the insert chain, e.g.
                // "FX DY EQ" — here: active inserts in processing order).
                let mut chain = String::from("FX");
                let t = app.project.track(track_id);
                if t.map(|t| t.comp.enabled).unwrap_or(false) {
                    chain.push_str(" MB");
                }
                if t.map(|t| t.dynamics.enabled).unwrap_or(false) {
                    chain.push_str(" DY");
                }
                if t.map(|t| t.eq.enabled && !t.eq.is_neutral()).unwrap_or(false) {
                    chain.push_str(" EQ");
                }
                let fx_active = chain.len() > 2;
                ui.label(
                    RichText::new(chain)
                        .monospace()
                        .size(9.0)
                        .color(if fx_active { Palette::ACCENT } else { Palette::TEXT_FAINT }),
                );
            });

            // Mini graphics previews (Fairlight-style): EQ response curve
            // and dynamics transfer curve for this track.
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                let half = (width - 26.0) / 2.0;
                let eq = app.project.track(track_id).map(|t| t.eq.clone());
                let dyst = app.project.track(track_id).map(|t| t.dynamics.clone());
                if let Some(eq) = &eq {
                    mini_eq_curve(ui, eq, half, 24.0);
                }
                if let Some(dyst) = &dyst {
                    mini_dyn_curve(ui, dyst, half, 24.0);
                }
            });

            // Pan knob.
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_space((width - 40.0) / 2.0 - 10.0);
                let mut pan = app.project.track(track_id).map(|t| t.pan).unwrap_or(0.0);
                if knob(ui, egui::Id::new(("mix_pan", track_id)), &mut pan, -1.0, 1.0, 0.0, 40.0, "") {
                    app.push_undo("mix");
                    if let Some(t) = app.project.track_mut(track_id) {
                        t.pan = pan;
                    }
                    app.mark_dirty();
                }
            });
            ui.horizontal(|ui| {
                ui.add_space((width - 40.0) / 2.0 - 10.0);
                let pan = app.project.track(track_id).map(|t| t.pan).unwrap_or(0.0);
                let label = if pan.abs() < 0.005 {
                    "C".to_string()
                } else if pan > 0.0 {
                    format!("R{:.0}", pan * 100.0)
                } else {
                    format!("L{:.0}", -pan * 100.0)
                };
                ui.label(RichText::new(label).small().monospace().color(Palette::TEXT_DIM));
            });

            // SOLO / MUTE / RECORD-ARM.
            ui.horizontal(|ui| {
                if strip_button(ui, "SOLO", solo, "Solo this track", SOLO_COLOR).clicked() {
                    app.push_undo("mix");
                    if let Some(t) = app.project.track_mut(track_id) {
                        t.solo = !t.solo;
                    }
                    app.mark_dirty();
                }
                if strip_button(ui, "MUTE", muted, "Mute this track", MUTE_COLOR).clicked() {
                    app.push_undo("mix");
                    if let Some(t) = app.project.track_mut(track_id) {
                        t.muted = !t.muted;
                    }
                    app.mark_dirty();
                }
            });
            ui.horizontal(|ui| {
                let armed = app.project.track(track_id).map(|t| t.record_arm).unwrap_or(false);
                if strip_button(ui, "● R", armed, "Record-arm (state only — no recorder yet)", ARM_COLOR).clicked() {
                    if let Some(t) = app.project.track_mut(track_id) {
                        t.record_arm = !t.record_arm;
                    }
                    app.mark_dirty();
                }
                let gr = app.audio.track_dynamics_gr(track_index);
                ui.label(
                    RichText::new(format!("GR {gr:+.1}"))
                        .monospace()
                        .size(9.0)
                        .color(if gr < -0.05 { Palette::ACCENT } else { Palette::TEXT_FAINT }),
                );
            });

            // Fader + meter.
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                let mut gain = app.project.track(track_id).map(|t| t.volume).unwrap_or(1.0);
                if fader(ui, egui::Id::new(("mix_fader", track_id)), &mut gain, fader_h) {
                    app.push_undo("mix");
                    if let Some(t) = app.project.track_mut(track_id) {
                        t.volume = gain;
                    }
                    app.mark_dirty();
                }
                let (level, hold) = app.meter_view(track_index, false);
                let mut hold = hold;
                meter(ui, level, &mut hold, fader_h, 8.0);
                app.set_meter_hold(track_index, false, hold);
            });

            // dB readout.
            let gain = app.project.track(track_id).map(|t| t.volume).unwrap_or(1.0);
            let db = gain_to_db(gain);
            ui.horizontal(|ui| {
                ui.add_space((width - 64.0) / 2.0 - 4.0);
                let text = if db <= -59.5 { "−∞ dB".to_string() } else { format!("{db:+.1} dB") };
                ui.label(
                    RichText::new(text)
                        .monospace()
                        .size(10.0)
                        .color(if db.abs() < 0.05 { Palette::ACCENT } else { Palette::TEXT }),
                );
            });
            });
        });
}

fn master_strip(app: &mut VeditApp, ui: &mut egui::Ui) {
    let width = 112.0;
    let fader_h = 196.0;
    egui::Frame::none()
        .fill(Color32::from_rgb(26, 26, 29))
        .stroke(egui::Stroke::new(1.0_f32, Palette::ACCENT_DIM))
        .inner_margin(8.0)
        .rounding(6.0)
        .show(ui, |ui| {
            ui.set_width(width);
            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {

            ui.label(RichText::new("MASTER").strong().size(12.0).color(Palette::ACCENT));
            ui.add_space(2.0);
            ui.label(
                RichText::new(format!(
                    "peak {:.1} dB",
                    gain_to_db(app.audio.master_level().max(1e-5) as f64)
                ))
                .monospace()
                .size(9.0)
                .color(Palette::TEXT_DIM),
            );

            // Control Room — BS.1770-4 loudness (LUFS) on the master bus.
            ui.add_space(4.0);
            ui.label(RichText::new("CONTROL ROOM").size(8.5).color(Palette::TEXT_FAINT));
            let (m, i) = app.audio.master_loudness();
            let lufs = |v: f32| {
                if v < -69.0 { "—".to_string() } else { format!("{v:.1}") }
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new("M").size(9.0).color(Palette::TEXT_DIM));
                ui.label(
                    RichText::new(format!("{} LUFS", lufs(m)))
                        .monospace()
                        .size(10.0)
                        .color(Palette::ACCENT),
                );
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("I").size(9.0).color(Palette::TEXT_DIM));
                ui.label(
                    RichText::new(format!("{} LUFS", lufs(i)))
                        .monospace()
                        .size(10.0)
                        .color(Palette::TEXT),
                );
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                let mut gain = app.project.master_volume;
                if fader(ui, egui::Id::new("master_fader"), &mut gain, fader_h) {
                    app.push_undo("mix");
                    app.project.master_volume = gain;
                    app.mark_dirty();
                }
                let (level, hold) = app.meter_view(0, true);
                let mut hold = hold;
                meter(ui, level, &mut hold, fader_h, 8.0);
                app.set_meter_hold(0, true, hold);
            });

            let db = gain_to_db(app.project.master_volume);
            ui.horizontal(|ui| {
                ui.add_space((width - 64.0) / 2.0 - 2.0);
                let text = if db <= -59.5 { "−∞ dB".to_string() } else { format!("{db:+.1} dB") };
                ui.label(
                    RichText::new(text)
                        .monospace()
                        .size(10.0)
                        .color(if db.abs() < 0.05 { Palette::ACCENT } else { Palette::TEXT }),
                );
            });
            });
        });
}

// ---------------------------------------------------------------------------
// FAIRLIGHT SIDEBAR — FX library (real inserts) + Index (Markers / Tracks).
// ---------------------------------------------------------------------------

fn fairlight_sidebar(app: &mut VeditApp, ctx: &egui::Context) {
    SidePanel::left("fairlight_sidebar")
        .resizable(true)
        .default_width(250.0)
        .min_width(196.0)
        .width_range(190.0..=400.0)
        .show(ctx, |ui| {
            panel_header(ui, "LIBRARY  ·  INDEX");
            ScrollArea::vertical().show(ui, |ui| {
                // ---- Fairlight FX (the inserts VEDIT actually has) ------
                ui.label(
                    RichText::new("FAIRLIGHT FX")
                        .size(9.5)
                        .color(Palette::TEXT_FAINT),
                );
                match app.fairlight_target_track() {
                    Some(tid) => {
                        let t = app.project.track(tid);
                        let eq_on = t.map(|t| t.eq.enabled && !t.eq.is_neutral()).unwrap_or(false);
                        let dyn_on = t.map(|t| t.dynamics.enabled).unwrap_or(false);
                        let comp_on = t.map(|t| t.comp.enabled).unwrap_or(false);
                        let track_name =
                            app.project.track(tid).map(|t| t.name.clone()).unwrap_or_default();
                        ui.label(
                            RichText::new(format!("Target · {track_name}"))
                                .small()
                                .color(Palette::TEXT_DIM),
                        );
                        ui.add_space(2.0);
                        if fx_row(ui, "Fairlight EQ", "6-band parametric equalizer", eq_on) {
                            app.eq_dialog_track = if app.eq_dialog_track == Some(tid) {
                                None
                            } else {
                                Some(tid)
                            };
                        }
                        if fx_row(ui, "Dynamics", "Gate → compressor → limiter", dyn_on) {
                            app.dyn_dialog_track = if app.dyn_dialog_track == Some(tid) {
                                None
                            } else {
                                Some(tid)
                            };
                        }
                        if fx_row(ui, "Multiband Comp", "3-band compressor", comp_on) {
                            app.comp_dialog_track = if app.comp_dialog_track == Some(tid) {
                                None
                            } else {
                                Some(tid)
                            };
                        }
                        if fx_row(ui, "Normalize −1 dBFS", "Loudest clip on the track → −1 dBFS", false) {
                            app.normalize_track(tid);
                        }
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                "Chorus, Delay, De-Hummer, Echo…\nare on the VEDIT roadmap.",
                            )
                            .small()
                            .color(Palette::TEXT_FAINT),
                        );
                    }
                    None => {
                        ui.label(
                            RichText::new("No audio tracks — add one on the timeline.")
                                .small()
                                .color(Palette::TEXT_FAINT),
                        );
                    }
                }

                ui.add_space(6.0);
                ui.separator();

                // ---- Index: Markers | Tracks -------------------------------
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut app.fairlight_index_tab,
                        0,
                        RichText::new("Markers").size(12.0),
                    );
                    ui.selectable_value(
                        &mut app.fairlight_index_tab,
                        1,
                        RichText::new("Tracks").size(12.0),
                    );
                });
                ui.add_space(2.0);
                if app.fairlight_index_tab == 1 {
                    tracks_index(app, ui);
                } else {
                    markers_index(app, ui);
                }
            });
        });
}

/// One library row: name + active dot. Returns true when clicked.
fn fx_row(ui: &mut egui::Ui, name: &str, tip: &str, active: bool) -> bool {
    ui.horizontal(|ui| {
        let clicked = ui
            .selectable_label(active, RichText::new(name).size(12.0))
            .on_hover_text(tip)
            .clicked();
        if active {
            ui.label(RichText::new("●").size(8.0).color(Palette::ACCENT));
        }
        clicked
    })
    .inner
}

/// Index → Markers: numbered table (n°, frame, name, TC, duration) with
/// click-to-jump and per-row delete.
fn markers_index(app: &mut VeditApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if ui
            .small_button("+ Add at playhead")
            .on_hover_text("Add a marker at the playhead (M)")
            .clicked()
        {
            app.add_marker_action();
        }
        ui.label(
            RichText::new(format!("{} total", app.project.markers.len()))
                .small()
                .color(Palette::TEXT_FAINT),
        );
    });
    if app.project.markers.is_empty() {
        ui.label(
            RichText::new("No markers yet — press M or click a ruler flag position.")
                .small()
                .color(Palette::TEXT_FAINT),
        );
        return;
    }
    let fps = app.project.fps;
    let markers: Vec<(u64, f64, String)> = app
        .project
        .markers
        .iter()
        .map(|m| (m.id, m.t, m.name.clone()))
        .collect();
    egui::Grid::new("marker_index")
        .num_columns(6)
        .spacing([6.0, 3.0])
        .striped(true)
        .show(ui, |ui| {
            let head = |ui: &mut egui::Ui, s: &str| {
                ui.label(RichText::new(s).size(9.0).color(Palette::TEXT_FAINT));
            };
            head(ui, "n°");
            head(ui, "Frame");
            head(ui, "Name");
            head(ui, "TC In");
            head(ui, "Dur");
            ui.end_row();

            for (i, (id, t, name)) in markers.iter().enumerate() {
                ui.label(
                    RichText::new(format!("{}", i + 1))
                        .small()
                        .monospace()
                        .color(Palette::TEXT_DIM),
                );
                ui.label(
                    RichText::new(format!("{}", (*t * fps).round() as i64))
                        .small()
                        .monospace(),
                );
                if ui
                    .selectable_label(false, RichText::new(name).size(11.0))
                    .on_hover_text("Click to jump the playhead to this marker")
                    .clicked()
                {
                    app.seek_to(*t);
                }
                ui.label(
                    RichText::new(format_timecode(*t, fps))
                        .small()
                        .monospace()
                        .color(Palette::TEXT_DIM),
                );
                ui.label(
                    RichText::new("0.0")
                        .small()
                        .monospace()
                        .color(Palette::TEXT_FAINT),
                );
                let del = ui.add(
                    egui::Button::new(RichText::new("×").size(9.0).color(Palette::TEXT_DIM))
                        .fill(Palette::SURFACE)
                        .small(),
                );
                if del.clicked() {
                    app.push_undo("marker");
                    app.project.remove_marker(*id);
                    app.mark_dirty();
                }
                del.on_hover_text("Delete marker");
                ui.end_row();
            }
        });
}

/// Index → Tracks: every track with kind, clip count and M/S/R state.
fn tracks_index(app: &mut VeditApp, ui: &mut egui::Ui) {
    let tracks: Vec<(String, TrackKind, bool, bool, bool, usize)> = app
        .project
        .tracks
        .iter()
        .map(|t| {
            (
                t.name.clone(),
                t.kind,
                t.muted,
                t.solo,
                t.record_arm,
                t.clips.len(),
            )
        })
        .collect();
    egui::Grid::new("tracks_index")
        .num_columns(4)
        .spacing([8.0, 3.0])
        .striped(true)
        .show(ui, |ui| {
            for (name, kind, muted, solo, armed, n_clips) in tracks {
                let dot = match kind {
                    TrackKind::Video => Palette::CLIP_VIDEO_BAR,
                    TrackKind::Audio => Palette::CLIP_AUDIO_BAR,
                };
                ui.label(RichText::new("●").size(9.0).color(dot));
                ui.label(RichText::new(name).size(11.5));
                ui.label(
                    RichText::new(format!("{n_clips} clips"))
                        .small()
                        .color(Palette::TEXT_FAINT),
                );
                let mut states = String::new();
                if muted {
                    states.push('M');
                }
                if solo {
                    states.push('S');
                }
                if armed {
                    states.push('R');
                }
                let empty = states.is_empty();
                ui.label(
                    RichText::new(if empty { "—".to_string() } else { states })
                        .small()
                        .monospace()
                        .color(if empty { Palette::TEXT_FAINT } else { Palette::ACCENT }),
                );
                ui.end_row();
            }
        });
}

// ---------------------------------------------------------------------------
// MINI STRIP GRAPHICS — tiny EQ response + dynamics transfer previews.
// ---------------------------------------------------------------------------

fn mini_eq_curve(ui: &mut egui::Ui, eq: &vedit_core::dsp::EqState, w: f32, h: f32) {
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::new(w, h), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 2.0, Color32::from_rgb(22, 22, 25));
    p.rect_stroke(rect, 2.0, egui::Stroke::new(0.5_f32, Palette::LINE));
    let active = eq.enabled && !eq.is_neutral();
    let color = if active { Palette::ACCENT } else { Palette::TEXT_FAINT };
    let n = 32;
    let pts: Vec<egui::Pos2> = (0..=n)
        .map(|i| {
            let f = 20.0 * (1000.0_f64).powf(i as f64 / n as f64); // 20 Hz…20 kHz, log
            let db = eq.response_db(f, 48_000.0).clamp(-18.0, 18.0);
            egui::Pos2::new(
                rect.left() + w * i as f32 / n as f32,
                rect.center().y - (db / 18.0) as f32 * (h * 0.42),
            )
        })
        .collect();
    p.add(egui::Shape::line(pts, egui::Stroke::new(1.2_f32, color)));
    resp.on_hover_text("EQ response (6-band)");
}

fn mini_dyn_curve(ui: &mut egui::Ui, st: &vedit_core::dsp::DynamicsState, w: f32, h: f32) {
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::new(w, h), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 2.0, Color32::from_rgb(22, 22, 25));
    p.rect_stroke(rect, 2.0, egui::Stroke::new(0.5_f32, Palette::LINE));
    let active = st.enabled;
    let color = if active { Palette::CLIP_AUDIO_BAR } else { Palette::TEXT_FAINT };
    let n = 32;
    let pts: Vec<egui::Pos2> = (0..=n)
        .map(|i| {
            let in_db = -60.0 + 60.0 * i as f64 / n as f64;
            let out_db = vedit_core::dsp::dynamics_transfer_db(st, in_db).clamp(-60.0, 0.0);
            egui::Pos2::new(
                rect.left() + w * i as f32 / n as f32,
                rect.bottom() - ((out_db + 60.0) / 60.0) as f32 * h,
            )
        })
        .collect();
    p.add(egui::Shape::line(pts, egui::Stroke::new(1.2_f32, color)));
    resp.on_hover_text("Dynamics transfer (gate → comp → limiter)");
}
