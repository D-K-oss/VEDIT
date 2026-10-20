//! Audio mixer panel (v0.2.0): per-track fader, pan, EQ, mute/solo + master.
//!
//! Live in preview: clip/track volume, mute, solo, master. Export-side only:
//! pan and the 3-band EQ (rodio cannot process those in real time).

use eframe::egui::{self, Button, Grid, RichText, ScrollArea, Slider};

use vedit_core::project::TrackKind;

use crate::app::VeditApp;
use crate::theme::Palette;

pub(crate) fn mixer(app: &mut VeditApp, ctx: &egui::Context) {
    if !app.mixer_open {
        return;
    }
    egui::SidePanel::left("mixer_panel")
        .resizable(true)
        .default_width(270.0)
        .min_width(220.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Mixer");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").on_hover_text("Close mixer").clicked() {
                        app.mixer_open = false;
                    }
                });
            });
            ui.separator();

            ScrollArea::vertical().show(ui, |ui| {
                // Snapshot tracks (id, name, kind, muted, solo) for editing.
                let tracks: Vec<(u64, String, TrackKind, bool, bool)> = app
                    .project
                    .tracks
                    .iter()
                    .map(|t| (t.id, t.name.clone(), t.kind, t.muted, t.solo))
                    .collect();

                for (id, name, kind, muted, solo) in tracks {
                    ui.group(|ui| {
                        ui.set_width(ui.available_width() - 4.0);
                        ui.horizontal(|ui| {
                            let dot = match kind {
                                TrackKind::Video => Palette::CLIP_VIDEO_BAR,
                                TrackKind::Audio => Palette::CLIP_AUDIO_BAR,
                            };
                            let dot_x = ui.cursor().left() + 3.0;
                            let dot_y = ui.cursor().center().y;
                            ui.painter().circle_filled(egui::Pos2::new(dot_x, dot_y), 3.0, dot);
                            ui.add_space(10.0);
                            ui.label(RichText::new(&name).strong());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let solo_btn = Button::new(RichText::new("S").size(9.0))
                                    .small()
                                    .fill(if solo { color32_fill_solo() } else { Palette::SURFACE });
                                if ui.add(solo_btn).on_hover_text("Solo").clicked() {
                                    app.push_undo("mix");
                                    if let Some(t) = app.project.track_mut(id) {
                                        t.solo = !t.solo;
                                    }
                                    app.mark_dirty();
                                }
                                let mute_btn = Button::new(RichText::new("M").size(9.0))
                                    .small()
                                    .fill(if muted { color32_fill_mute() } else { Palette::SURFACE });
                                if ui.add(mute_btn).on_hover_text("Mute").clicked() {
                                    app.push_undo("mix");
                                    if let Some(t) = app.project.track_mut(id) {
                                        t.muted = !t.muted;
                                    }
                                    app.mark_dirty();
                                }
                            });
                        });

                        let mut vol = app.project.track(id).map(|t| t.volume).unwrap_or(1.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Vol").small().color(Palette::TEXT_DIM));
                            if ui
                                .add(Slider::new(&mut vol, 0.0..=2.0).show_value(false))
                                .changed()
                            {
                                app.push_undo("mix");
                                if let Some(t) = app.project.track_mut(id) {
                                    t.volume = vol;
                                }
                                app.mark_dirty();
                            }
                            ui.label(
                                RichText::new(format!("{:.0}%", vol * 100.0))
                                    .small()
                                    .monospace(),
                            );
                        });

                        let mut pan = app.project.track(id).map(|t| t.pan).unwrap_or(0.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Pan").small().color(Palette::TEXT_DIM));
                            if ui
                                .add(Slider::new(&mut pan, -1.0..=1.0).show_value(false))
                                .changed()
                            {
                                app.push_undo("mix");
                                if let Some(t) = app.project.track_mut(id) {
                                    t.pan = pan;
                                }
                                app.mark_dirty();
                            }
                            ui.label(RichText::new(pan_label(pan)).small().monospace());
                        });

                        // EQ (export-side), collapsed by default.
                        let (mut b, mut m, mut tr) = app
                            .project
                            .track(id)
                            .map(|t| (t.eq_bass, t.eq_mid, t.eq_treble))
                            .unwrap_or((0.0, 0.0, 0.0));
                        egui::CollapsingHeader::new(RichText::new("EQ").small())
                            .id_salt(("mix_eq", id))
                            .show(ui, |ui| {
                                Grid::new(("mix_eq_grid", id))
                                    .num_columns(2)
                                    .spacing([8.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label(RichText::new("Bass").small());
                                        if ui
                                            .add(Slider::new(&mut b, -12.0..=12.0).suffix(" dB"))
                                            .changed()
                                        {
                                            app.push_undo("mix");
                                            if let Some(t) = app.project.track_mut(id) {
                                                t.eq_bass = b;
                                            }
                                            app.mark_dirty();
                                        }
                                        ui.end_row();
                                        ui.label(RichText::new("Mid").small());
                                        if ui
                                            .add(Slider::new(&mut m, -12.0..=12.0).suffix(" dB"))
                                            .changed()
                                        {
                                            app.push_undo("mix");
                                            if let Some(t) = app.project.track_mut(id) {
                                                t.eq_mid = m;
                                            }
                                            app.mark_dirty();
                                        }
                                        ui.end_row();
                                        ui.label(RichText::new("Treble").small());
                                        if ui
                                            .add(Slider::new(&mut tr, -12.0..=12.0).suffix(" dB"))
                                            .changed()
                                        {
                                            app.push_undo("mix");
                                            if let Some(t) = app.project.track_mut(id) {
                                                t.eq_treble = tr;
                                            }
                                            app.mark_dirty();
                                        }
                                        ui.end_row();
                                    });
                            });
                    });
                    ui.add_space(2.0);
                }

                // Master strip.
                ui.group(|ui| {
                    ui.set_width(ui.available_width() - 4.0);
                    ui.label(RichText::new("MASTER").strong());
                    let mut mv = app.project.master_volume;
                    ui.horizontal(|ui| {
                        if ui
                            .add(Slider::new(&mut mv, 0.0..=2.0).show_value(false))
                            .changed()
                        {
                            app.push_undo("mix");
                            app.project.master_volume = mv;
                            app.mark_dirty();
                        }
                        ui.label(
                            RichText::new(format!("{:.0}%", mv * 100.0))
                                .small()
                                .monospace(),
                        );
                    });
                });

                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Volume, mute, solo and master are live in the preview.\nPan and EQ are applied on export.",
                    )
                    .small()
                    .color(Palette::TEXT_DIM),
                );
            });
        });
}

fn pan_label(pan: f64) -> String {
    if pan.abs() < 0.005 {
        "C".into()
    } else if pan > 0.0 {
        format!("R{:.0}", pan * 100.0)
    } else {
        format!("L{:.0}", -pan * 100.0)
    }
}

fn color32_fill_solo() -> eframe::egui::Color32 {
    eframe::egui::Color32::from_rgb(48, 40, 24)
}

fn color32_fill_mute() -> eframe::egui::Color32 {
    eframe::egui::Color32::from_rgb(45, 28, 26)
}
