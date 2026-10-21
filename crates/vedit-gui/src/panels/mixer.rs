//! Audio mixer panel — a real console strip per audio track (and MASTER).
//!
//! Each strip: Normalize / EQ / Comp buttons, pan knob, SOLO + MUTE, a
//! vertical dB fader (−60…+12) with scale ticks, a live peak meter and a dB
//! readout. Fader, pan, mute, solo, EQ, compressor and master are ALL live
//! in the preview (software mixer on the audio thread); the same settings
//! drive the export filtergraph.
//!
//! Metering comes from the audio engine's per-bus peak atomics; the UI decays
//! the displayed level so it reads like a real meter.

use eframe::egui::{self, Color32, RichText, ScrollArea};

use vedit_core::project::TrackKind;

use crate::app::VeditApp;
use crate::panels::widgets::{fader, gain_to_db, knob, meter, strip_button};
use crate::theme::Palette;

const SOLO_COLOR: Color32 = Color32::from_rgb(48, 40, 24);
const MUTE_COLOR: Color32 = Color32::from_rgb(45, 28, 26);

pub(crate) fn mixer(app: &mut VeditApp, ctx: &egui::Context) {
    if !app.mixer_open {
        return;
    }
    // Decay meters every frame so bars fall even when audio goes idle.
    app.decay_meters();

    egui::SidePanel::left("mixer_panel")
        .resizable(true)
        .default_width(430.0)
        .min_width(300.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Mixer");
                ui.label(
                    RichText::new("live in preview · applied on export")
                        .small()
                        .color(Palette::TEXT_DIM),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").on_hover_text("Close mixer").clicked() {
                        app.mixer_open = false;
                    }
                });
            });
            ui.separator();

            ScrollArea::horizontal().show(ui, |ui| {
                ui.horizontal(|ui| {
                    let tracks: Vec<(usize, u64, String, bool, bool)> = app
                        .project
                        .tracks
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| t.kind == TrackKind::Audio)
                        .map(|(i, t)| (i, t.id, t.name.clone(), t.muted, t.solo))
                        .collect();

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
                .color(Palette::TEXT_DIM),
            );
        });
}

/// One channel strip.
fn strip(app: &mut VeditApp, ui: &mut egui::Ui, track_index: usize, track_id: u64, name: &str, muted: bool, solo: bool) {
    let width = 104.0;
    egui::Frame::none()
        .fill(Palette::PANEL)
        .stroke(egui::Stroke::new(1.0_f32, Palette::LINE))
        .inner_margin(6.0)
        .rounding(4.0)
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
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                let mut gain = app.project.track(track_id).map(|t| t.volume).unwrap_or(1.0);
                if fader(ui, egui::Id::new(("mix_fader", track_id)), &mut gain, 122.0) {
                    app.push_undo("mix");
                    if let Some(t) = app.project.track_mut(track_id) {
                        t.volume = gain;
                    }
                    app.mark_dirty();
                }
                let (level, hold) = app.meter_view(track_index, false);
                let mut hold = hold;
                meter(ui, level, &mut hold, 122.0, 8.0);
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
    let width = 104.0;
    egui::Frame::none()
        .fill(Color32::from_rgb(26, 26, 29))
        .stroke(egui::Stroke::new(1.0_f32, Palette::ACCENT_DIM))
        .inner_margin(6.0)
        .rounding(4.0)
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

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                let mut gain = app.project.master_volume;
                if fader(ui, egui::Id::new("master_fader"), &mut gain, 128.0) {
                    app.push_undo("mix");
                    app.project.master_volume = gain;
                    app.mark_dirty();
                }
                let (level, hold) = app.meter_view(0, true);
                let mut hold = hold;
                meter(ui, level, &mut hold, 128.0, 8.0);
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
