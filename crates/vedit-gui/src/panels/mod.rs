//! Panel modules.

pub mod audio_dialogs;
pub mod export_dialog;
pub mod grade_panel;
pub mod inspector;
pub mod media_library;
pub mod mixer;
pub mod preview;
pub mod scopes;
pub mod settings_dialog;
pub mod timeline_ui;
pub mod widgets;

pub(crate) use export_dialog::export_dialog;
pub(crate) use grade_panel::grade_page;
pub(crate) use inspector::inspector;
pub(crate) use media_library::media_library;
pub(crate) use mixer::mixer_page;
pub(crate) use preview::preview;
pub(crate) use scopes::{scopes_panel, scopes_window};
pub(crate) use settings_dialog::settings_dialog;
pub(crate) use timeline_ui::timeline;
pub(crate) use audio_dialogs::audio_dialogs;

use eframe::egui::{self, Align, Button, Layout, RichText, TextEdit, TopBottomPanel, Window};
use std::time::Duration;

use crate::app::{FPS_PRESETS, RES_PRESETS, VeditApp, Workspace};
use crate::theme::Palette;

// ---------------------------------------------------------------------------

/// Top bar: file actions, undo/redo (with jumpable history), the WORKSPACE
/// switcher (Edit · Color · Audio — the "tabs" that change the whole
/// workplace) and the export button.
pub(crate) fn top_bar(app: &mut VeditApp, ctx: &egui::Context) {
    TopBottomPanel::top("top_bar").exact_height(48.0).show(ctx, |ui| {
        ui.horizontal_centered(|ui| {
            ui.add_space(12.0);
            ui.label(RichText::new("VEDIT").strong().size(17.0).color(Palette::ACCENT));
            ui.label(RichText::new("0.3.0").small().color(Palette::TEXT_FAINT));
            ui.add_space(8.0);
            ui.separator();

            if ui.button("New").clicked() {
                app.new_dialog.open = true;
            }
            if ui.button("Open…").on_hover_text("Ctrl+O").clicked() {
                app.open_project_dialog();
            }
            if ui.button("Save").on_hover_text("Ctrl+S").clicked() {
                app.save_project(false);
            }
            if ui.button("Import…").on_hover_text("Ctrl+I").clicked() {
                app.import_media_dialog();
            }

            ui.separator();
            // Undo / redo + jumpable history.
            if ui
                .add_enabled(app.history.can_undo(), Button::new("⟲"))
                .on_hover_text("Undo (Ctrl+Z)")
                .clicked()
            {
                app.do_undo();
            }
            if ui
                .add_enabled(app.history.can_redo(), Button::new("⟳"))
                .on_hover_text("Redo (Ctrl+Y)")
                .clicked()
            {
                app.do_redo();
            }
            history_dropdown(app, ui);

            ui.separator();

            // THE WORKSPACE TABS — each button switches the entire workplace.
            for w in Workspace::ALL {
                let active = app.workspace == w;
                let btn = Button::new(
                    RichText::new(w.label()).strong().color(if active {
                        Palette::ACCENT_TEXT
                    } else {
                        Palette::TEXT
                    }),
                )
                .fill(if active { Palette::ACCENT } else { Palette::SURFACE })
                .min_size(egui::vec2(74.0, 24.0));
                if ui.add(btn).on_hover_text(w.hint()).clicked() {
                    app.workspace = w;
                }
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let export_btn = Button::new(
                    RichText::new("Export").strong().color(Palette::ACCENT_TEXT),
                )
                .fill(Palette::ACCENT);
                if ui.add(export_btn).on_hover_text("Ctrl+E").clicked() {
                    app.export.open = true;
                }
                if ui.button("⚙").on_hover_text("Settings").clicked() {
                    app.settings_open = true;
                }
                let title = format!(
                    "{}{}   ·   {} × {}   ·   {:.3} fps",
                    app.project.name,
                    if app.dirty { "  •" } else { "" },
                    app.project.width,
                    app.project.height,
                    app.project.fps
                );
                ui.label(RichText::new(title).color(Palette::TEXT_DIM));
            });
        });
    });
}

/// Jumpable undo history (Venturi: "unlimited undo with a jumpable
/// history"): pick a step from the dropdown to travel back to it.
fn history_dropdown(app: &mut VeditApp, ui: &mut egui::Ui) {
    let n = app.history.undo_labels().len();
    let label = if n == 0 { "History".to_string() } else { format!("History ({n})") };
    ui.menu_button(label, |ui| {
        if n == 0 {
            ui.label(RichText::new("No steps yet").small().color(Palette::TEXT_FAINT));
        }
        let labels = app.history.undo_labels().to_vec();
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            // Newest step first (top of the list).
            for (i, l) in labels.iter().enumerate().rev() {
                let steps_back = labels.len() - i;
                if ui
                    .selectable_label(false, format!("{steps_back:>3}  ·  {l}"))
                    .clicked()
                {
                    if let Some(p) = app.history.undo_n(&app.project, steps_back) {
                        app.project = p;
                        app.selected = None;
                        app.dirty = true;
                        app.request_preview_frame();
                        app.notify(format!("Jumped back {steps_back} step(s)"));
                    }
                    ui.close_menu();
                }
            }
        });
    });
}

// ---------------------------------------------------------------------------

pub(crate) fn status_bar(app: &mut VeditApp, ctx: &egui::Context) {
    TopBottomPanel::bottom("status_bar").exact_height(26.0).show(ctx, |ui| {
        ui.horizontal_centered(|ui| {
            ui.add_space(10.0);
            let file = match &app.project_path {
                Some(p) => p.display().to_string(),
                None => "unsaved project".to_string(),
            };
            let dirty_mark = if app.dirty { "  •" } else { "" };
            ui.label(RichText::new(format!("{file}{dirty_mark}")).small().color(Palette::TEXT_FAINT));

            if let Some((msg, at)) = &app.status {
                if at.elapsed() < Duration::from_secs(5) {
                    ui.label(RichText::new(msg).small().color(Palette::ACCENT));
                }
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new("VEDIT 0.3.0").small().color(Palette::TEXT_FAINT));
                match (&app.locator, &app.ffmpeg_version) {
                    (Some(l), Some(v)) => {
                        ui.label(
                            RichText::new(format!("{} · {}", v, l.ffmpeg().display()))
                                .small()
                                .color(Palette::OK),
                        );
                    }
                    _ => {
                        ui.label(
                            RichText::new("ffmpeg NOT FOUND — media features disabled")
                                .small()
                                .color(Palette::DANGER),
                        );
                    }
                }
            });
        });
    });
}

// ---------------------------------------------------------------------------

pub(crate) fn new_project_dialog(app: &mut VeditApp, ctx: &egui::Context) {
    if !app.new_dialog.open {
        return;
    }
    let mut open = true;
    Window::new("New Project")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .min_width(420.0)
        .show(ctx, |ui| {
            let res_labels: Vec<&str> = RES_PRESETS.iter().map(|(l, _, _)| *l).collect();
            egui::Grid::new("np_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.add(
                        TextEdit::singleline(&mut app.new_dialog.name)
                            .desired_width(240.0),
                    );
                    ui.end_row();

                    ui.label("Resolution");
                    egui::ComboBox::from_id_salt("np_res")
                        .selected_text(res_labels[app.new_dialog.res_index])
                        .width(240.0)
                        .show_ui(ui, |ui| {
                            for (i, label) in res_labels.iter().enumerate() {
                                ui.selectable_value(&mut app.new_dialog.res_index, i, *label);
                            }
                        });
                    ui.end_row();

                    ui.label("Frame rate");
                    egui::ComboBox::from_id_salt("np_fps")
                        .selected_text(format!("{:.3} fps", FPS_PRESETS[app.new_dialog.fps_index]))
                        .width(240.0)
                        .show_ui(ui, |ui| {
                            for (i, f) in FPS_PRESETS.iter().enumerate() {
                                ui.selectable_value(
                                    &mut app.new_dialog.fps_index,
                                    i,
                                    format!("{f:.3} fps"),
                                );
                            }
                        });
                    ui.end_row();
                });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    app.new_dialog.open = false;
                }
                if ui
                    .add(
                        Button::new(RichText::new("Create").strong().color(Palette::ACCENT_TEXT))
                            .fill(Palette::ACCENT),
                    )
                    .clicked()
                {
                    let (_, w, h) = RES_PRESETS[app.new_dialog.res_index];
                    let fps = FPS_PRESETS[app.new_dialog.fps_index];
                    let name = app.new_dialog.name.clone();
                    app.create_new_project(&name, w, h, fps);
                    app.new_dialog.open = false;
                }
            });
        });
    if !open {
        app.new_dialog.open = false;
    }
}
