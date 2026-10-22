//! Audio workspace: a full console page — channel strips per audio track
//! (EQ / compressor / normalize / pan / solo / mute / fader / meter) and a
//! MASTER strip. Live in preview, applied on export.

use eframe::egui::{self, Color32, RichText, ScrollArea, SidePanel};

use vedit_core::project::TrackKind;

use crate::app::VeditApp;
use crate::panels::widgets::{fader, gain_to_db, knob, meter, strip_button};
use crate::theme::{panel_header, Palette};

const SOLO_COLOR: Color32 = Color32::from_rgb(48, 40, 24);
const MUTE_COLOR: Color32 = Color32::from_rgb(45, 28, 26);

/// The Audio workspace page (whole left side of the window).
pub(crate) fn mixer_page(app: &mut VeditApp, ctx: &egui::Context) {
    // Decay meters every frame so bars fall even when audio goes idle.
    app.decay_meters();

    SidePanel::left("mixer_page")
        .resizable(true)
        .default_width(460.0)
        .min_width(300.0)
        .width_range(300.0..=760.0)
        .show(ctx, |ui| {
            panel_header(ui, "AUDIO MIXER  ·  live in preview · applied on export");

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

            // EQ / Comp / Normalize buttons.
            ui.horizontal(|ui| {
                let eq_active = app
                    .project
                    .track(track_id)
                    .map(|t| t.eq.enabled && !t.eq.is_neutral())
                    .unwrap_or(false);
                let comp_active = app
                    .project
                    .track(track_id)
                    .map(|t| t.comp.enabled)
                    .unwrap_or(false);
                if strip_button(ui, "EQ", eq_active, "6-band parametric equalizer", SOLO_COLOR).clicked() {
                    app.eq_dialog_track = if app.eq_dialog_track == Some(track_id) { None } else { Some(track_id) };
                }
                if strip_button(ui, "Comp", comp_active, "3-band multiband compressor", SOLO_COLOR).clicked() {
                    app.comp_dialog_track = if app.comp_dialog_track == Some(track_id) { None } else { Some(track_id) };
                }
            });
            ui.horizontal(|ui| {
                if strip_button(ui, "Norm", false, "Normalize: loudest clip on this track → −1 dBFS", SOLO_COLOR).clicked() {
                    app.normalize_track(track_id);
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

            // SOLO / MUTE.
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

            ui.add_space(6.0);
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
