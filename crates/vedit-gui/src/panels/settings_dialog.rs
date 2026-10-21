//! Settings dialog (v0.2.1): autosave, snapping, preview default, ffmpeg dir.

use eframe::egui::{self, Checkbox, DragValue, Grid, RichText, TextEdit, Window};

use crate::app::VeditApp;
use crate::theme::Palette;

pub(crate) fn settings_dialog(app: &mut VeditApp, ctx: &egui::Context) {
    if !app.settings_open {
        return;
    }
    let mut open = true;
    Window::new("Settings")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .min_width(440.0)
        .show(ctx, |ui| {
            let mut save_needed = false;

            ui.label(RichText::new("Project").strong().small());
            Grid::new("set_grid")
                .num_columns(2)
                .spacing([12.0, 7.0])
                .show(ui, |ui| {
                    ui.label("Autosave");
                    if ui.add(Checkbox::new(&mut app.settings.autosave_enabled, "enabled")).changed() {
                        save_needed = true;
                    }
                    ui.end_row();

                    ui.label("Autosave interval");
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                DragValue::new(&mut app.settings.autosave_minutes)
                                    .speed(1.0)
                                    .range(1..=120)
                                    .suffix(" min"),
                            )
                            .changed()
                        {
                            save_needed = true;
                        }
                    });
                    ui.end_row();

                    ui.label("Timeline snapping");
                    if ui
                        .add(Checkbox::new(&mut app.settings.snap_default, "on at startup"))
                        .changed()
                    {
                        app.tl.snap = app.settings.snap_default;
                        save_needed = true;
                    }
                    ui.end_row();

                    ui.label("Default preview quality");
                    let names = ["Full", "Half", "Quarter"];
                    let mut q = app.settings.preview_quality_default.min(2);
                    egui::ComboBox::from_id_salt("set_quality")
                        .selected_text(names[q])
                        .width(110.0)
                        .show_ui(ui, |ui| {
                            for (i, n) in names.iter().enumerate() {
                                ui.selectable_value(&mut q, i, *n);
                            }
                        });
                    if q != app.settings.preview_quality_default {
                        app.settings.preview_quality_default = q;
                        save_needed = true;
                    }
                    ui.end_row();
                });

            ui.add_space(8.0);
            ui.separator();
            ui.label(RichText::new("Media engine").strong().small());
            Grid::new("set_ff_grid")
                .num_columns(2)
                .spacing([12.0, 7.0])
                .show(ui, |ui| {
                    ui.label("ffmpeg folder");
                    let mut dir = app.settings.ffmpeg_dir.clone().unwrap_or_default();
                    if ui
                        .add(
                            TextEdit::singleline(&mut dir)
                                .hint_text("folder with ffmpeg + ffprobe (empty = auto)")
                                .desired_width(260.0),
                        )
                        .lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        apply_ffmpeg_dir(app, dir.trim());
                        save_needed = true;
                    } else {
                        app.settings.ffmpeg_dir = if dir.trim().is_empty() { None } else { Some(dir) };
                    }
                    ui.end_row();
                });
            ui.horizontal(|ui| {
                if ui.button("Re-detect").clicked() {
                    let dir = app.settings.ffmpeg_dir.clone().unwrap_or_default();
                    apply_ffmpeg_dir(app, &dir);
                }
                match (&app.locator, &app.ffmpeg_version) {
                    (Some(l), Some(v)) => {
                        ui.label(RichText::new(format!("{} · {}", v, l.ffmpeg().display())).small().color(Palette::OK));
                    }
                    _ => {
                        ui.label(RichText::new("ffmpeg NOT FOUND").small().color(Palette::DANGER));
                    }
                }
            });

            ui.add_space(10.0);
            ui.separator();
            ui.label(
                RichText::new("VEDIT 0.2.1 · settings persist in ~/.vedit/settings.json")
                    .small()
                    .color(Palette::TEXT_DIM),
            );

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("Done").strong().color(Palette::ACCENT_TEXT),
                        )
                        .fill(Palette::ACCENT),
                    )
                    .clicked()
                {
                    app.settings_open = false;
                    save_needed = true;
                }
            });

            if save_needed {
                app.settings.save();
            }
        });
    if !open {
        app.settings_open = false;
        app.settings.save();
    }
}

fn apply_ffmpeg_dir(app: &mut VeditApp, dir: &str) {
    app.settings.ffmpeg_dir = if dir.trim().is_empty() { None } else { Some(dir.to_string()) };
    let new_locator = match app.settings.ffmpeg_dir.as_deref() {
        Some(d) if !d.trim().is_empty() => {
            vedit_core::ffmpeg::FfmpegLocator::from_dir(std::path::Path::new(d.trim())).ok()
        }
        _ => vedit_core::ffmpeg::FfmpegLocator::discover().ok(),
    };
    match new_locator {
        Some(l) => {
            app.ffmpeg_version = Some(l.version());
            app.locator = Some(l);
            app.notify("ffmpeg located");
        }
        None => {
            app.notify("No ffmpeg/ffprobe pair found there");
        }
    }
}
