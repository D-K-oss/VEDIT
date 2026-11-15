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
pub(crate) use inspector::{inspector, keyframes_window};
pub(crate) use media_library::media_library;
pub(crate) use mixer::mixer_page;
pub(crate) use preview::preview;
pub(crate) use scopes::{scopes_panel, scopes_window};
pub(crate) use settings_dialog::settings_dialog;
pub(crate) use timeline_ui::timeline;
pub(crate) use audio_dialogs::audio_dialogs;

use eframe::egui::{self, Align, Button, FontId, Layout, RichText, TextEdit, TopBottomPanel, Vec2, Window};
use std::time::Duration;

use crate::app::{FPS_PRESETS, RES_PRESETS, VeditApp, Workspace};
use crate::theme::{mono, Palette};

// ---------------------------------------------------------------------------
// TOP BAR — file actions + undo/redo + project info + settings/export.
// (Workspace pages moved to the bottom PAGE BAR, DaVinci-style.)
// ---------------------------------------------------------------------------

pub(crate) fn top_bar(app: &mut VeditApp, ctx: &egui::Context) {
    TopBottomPanel::top("top_bar").exact_height(42.0).show(ctx, |ui| {
        ui.horizontal_centered(|ui| {
            ui.add_space(12.0);
            ui.label(RichText::new("VEDIT").strong().size(16.0).color(Palette::ACCENT));
            ui.label(RichText::new("0.4.0").small().color(Palette::TEXT_FAINT));
            ui.add_space(6.0);
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

// ---------------------------------------------------------------------------
// ACTION BAR — the DaVinci Edit-page toolbar: left tool strip (media pool,
// sync bin, transitions, titles, effects, keyframes + view toggles + search),
// center project/timeline/timecode, right quick actions.
// ---------------------------------------------------------------------------

fn small_tool_btn(ui: &mut egui::Ui, icon: &str, label: &str, active: bool, tip: &str) -> bool {
    let txt = RichText::new(format!("{icon} {label}")).size(11.5);
    let btn = Button::new(if active { txt.strong().color(Palette::ACCENT) } else { txt })
        .fill(if active { Palette::ACCENT_SOFT } else { Palette::SURFACE })
        .min_size(Vec2::new(0.0, 22.0));
    ui.add(btn).on_hover_text(tip).clicked()
}

pub(crate) fn action_bar(app: &mut VeditApp, ctx: &egui::Context) {
    TopBottomPanel::top("action_bar")
        .exact_height(36.0)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(8.0);

                // --- left: media/workspace tool strip -------------------
                if small_tool_btn(
                    ui,
                    "▤",
                    "Media Pool",
                    app.media_panel_open,
                    "Show/hide the media pool (Ctrl+I to import)",
                ) {
                    app.media_panel_open = !app.media_panel_open;
                }
                if small_tool_btn(ui, "⇄", "Sync Bin", false, "Sync Bin — not available in VEDIT yet") {
                    app.notify("Sync Bin is not available in VEDIT yet");
                }
                // Transitions menu (real controls for the selected clip).
                ui.menu_button("◐ Transitions", |ui| {
                    ui.set_min_width(220.0);
                    let has_sel = app.selected.is_some();
                    let label = if has_sel {
                        "Add cross dissolve — 0.5 s"
                    } else {
                        "Add cross dissolve (select a clip first)"
                    };
                    if ui.add_enabled(has_sel, Button::new(label)).clicked() {
                        set_transition_on_selected(app, 0.5);
                        ui.close_menu();
                    }
                    if ui.add_enabled(has_sel, Button::new("Add cross dissolve — 1 s")).clicked() {
                        set_transition_on_selected(app, 1.0);
                        ui.close_menu();
                    }
                    if ui.add_enabled(has_sel, Button::new("Clear transition")).clicked() {
                        clear_transition_on_selected(app);
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.label(RichText::new("Fades on selection").small().color(Palette::TEXT_FAINT));
                    if ui.add_enabled(has_sel, Button::new("Fade in — 0.5 s")).clicked() {
                        set_fade_on_selected(app, true, 0.5);
                        ui.close_menu();
                    }
                    if ui.add_enabled(has_sel, Button::new("Fade out — 0.5 s")).clicked() {
                        set_fade_on_selected(app, false, 0.5);
                        ui.close_menu();
                    }
                });
                if small_tool_btn(ui, "T", "Titles", false, "Add a title clip at the playhead") {
                    app.add_title_action();
                }
                // Effects menu (quick look effects on the selection).
                ui.menu_button("✶ Effects", |ui| {
                    ui.set_min_width(230.0);
                    let has_sel = app.selected.is_some();
                    if ui.add_enabled(has_sel, Button::new("Toggle Dynamic Zoom (push-in)")).clicked() {
                        app.toggle_dynamic_zoom();
                        ui.close_menu();
                    }
                    if ui.add_enabled(has_sel, Button::new("Copy attributes (Ctrl+C)")).clicked() {
                        app.copy_attrs_action();
                        ui.close_menu();
                    }
                    if ui.add_enabled(has_sel, Button::new("Paste attributes (Ctrl+V)")).clicked() {
                        app.paste_attrs_action();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Open Color page (grade + LUT)").clicked() {
                        app.workspace = Workspace::Color;
                        ui.close_menu();
                    }
                });
                if small_tool_btn(
                    ui,
                    "◆",
                    "Keyframes",
                    app.keyframes_open,
                    "Show the keyframe list for the selected clip",
                ) {
                    app.keyframes_open = !app.keyframes_open;
                }

                ui.separator();
                // View toggles: list / grid.
                if ui
                    .add(Button::new(RichText::new("☰").size(12.0)).small())
                    .on_hover_text("Media list view")
                    .clicked()
                {
                    app.media_view = crate::app::MediaView::List;
                }
                if ui
                    .add(Button::new(RichText::new("▦").size(12.0)).small())
                    .on_hover_text("Media thumbnail grid")
                    .clicked()
                {
                    app.media_view = crate::app::MediaView::Grid;
                }
                ui.add(
                    TextEdit::singleline(&mut app.media_filter)
                        .hint_text("Cerca…")
                        .desired_width(110.0),
                );

                // --- center: project · timeline · timecode ---------------
                ui.separator();
                ui.label(
                    RichText::new(&app.project.name)
                        .strong()
                        .size(12.5)
                        .color(Palette::TEXT),
                );
                ui.label(RichText::new("·").color(Palette::TEXT_FAINT));
                ui.label(RichText::new("Timeline 1").size(12.0).color(Palette::TEXT_DIM));
                ui.label(RichText::new("·").color(Palette::TEXT_FAINT));
                ui.label(mono(
                    vedit_core::media::format_timecode(app.playhead, app.project.fps),
                    Palette::ACCENT,
                ));

                // --- right: quick export · fullscreen · mixer · browse ---
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if small_tool_btn(ui, "◉", "Browse", app.workspace == Workspace::Media, "Media page (Alt+1)") {
                        app.workspace = Workspace::Media;
                    }
                    if small_tool_btn(ui, "‖", "Mixer", app.workspace == Workspace::Audio, "Fairlight mixer page (Alt+6)") {
                        app.workspace = Workspace::Audio;
                    }
                    if small_tool_btn(ui, "▢", "Fullscreen", app.fullscreen_preview, "Fullscreen preview — hide side panels") {
                        app.fullscreen_preview = !app.fullscreen_preview;
                    }
                    if small_tool_btn(ui, "↧", "Quick Export", false, "Open the export dialog (Ctrl+E)") {
                        app.export.open = true;
                    }
                });
            });
        });
}

/// Set a cross dissolve on the selected clip's tail (duration seconds).
fn set_transition_on_selected(app: &mut VeditApp, d: f64) {
    let Some((track_id, clip_id)) = app.selected else { return };
    let Some(mut c) = app.project.find_clip(track_id, clip_id).cloned() else { return };
    app.push_undo("transition");
    c.transition_out = Some(vedit_core::project::Transition {
        kind: vedit_core::project::TransitionKind::CrossDissolve,
        duration: d,
    });
    app.apply_clip_by_id(track_id, c);
    app.mark_dirty();
    app.notify(format!("Cross dissolve {d} s added"));
    app.request_preview_frame();
}

fn clear_transition_on_selected(app: &mut VeditApp) {
    let Some((track_id, clip_id)) = app.selected else { return };
    let Some(mut c) = app.project.find_clip(track_id, clip_id).cloned() else { return };
    if c.transition_out.is_none() {
        app.notify("No transition on this clip");
        return;
    }
    app.push_undo("transition");
    c.transition_out = None;
    app.apply_clip_by_id(track_id, c);
    app.mark_dirty();
    app.notify("Transition cleared");
    app.request_preview_frame();
}

fn set_fade_on_selected(app: &mut VeditApp, fade_in: bool, d: f64) {
    let Some((track_id, clip_id)) = app.selected else { return };
    let Some(mut c) = app.project.find_clip(track_id, clip_id).cloned() else { return };
    app.push_undo("fade");
    let d = d.min(c.duration * 0.5);
    if fade_in {
        c.fade_in = d;
    } else {
        c.fade_out = d;
    }
    app.apply_clip_by_id(track_id, c);
    app.mark_dirty();
    app.notify(if fade_in { "Fade in added" } else { "Fade out added" });
    app.request_preview_frame();
}

// ---------------------------------------------------------------------------
// PAGE BAR — bottom global navigation (Media · Cut · Edit · Fusion · Color ·
// Fairlight · Deliver). Active page gets the accent underline marker.
// ---------------------------------------------------------------------------

pub(crate) fn page_bar(app: &mut VeditApp, ctx: &egui::Context) {
    TopBottomPanel::bottom("page_bar")
        .exact_height(34.0)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(10.0);
                for w in Workspace::ALL {
                    page_button(app, ui, w);
                }
                ui.separator();
                deliver_button(app, ui);
            });
        });
}

fn page_button(app: &mut VeditApp, ui: &mut egui::Ui, w: Workspace) {
    let active = app.workspace == w;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(78.0, 26.0), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 4.0, Palette::SURFACE_HOVER);
    }
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        w.label(),
        FontId::proportional(12.5),
        if active { Palette::ACCENT } else { Palette::TEXT_DIM },
    );
    if active {
        // Red-ish underline marker like the reference app.
        p.rect_filled(
            egui::Rect::from_min_max(
                egui::Pos2::new(rect.left() + 8.0, rect.bottom() - 2.0),
                egui::Pos2::new(rect.right() - 8.0, rect.bottom() - 0.5),
            ),
            1.0,
            Palette::PLAYHEAD,
        );
    }
    if resp.clicked() {
        app.workspace = w;
    }
    resp.on_hover_text(w.hint());
}

fn deliver_button(app: &mut VeditApp, ui: &mut egui::Ui) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(78.0, 26.0), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 4.0, Palette::SURFACE_HOVER);
    }
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "Deliver",
        FontId::proportional(12.5),
        Palette::TEXT_DIM,
    );
    if resp.clicked() {
        app.export.open = true;
    }
    resp.on_hover_text("Render & deliver — open the export dialog (Ctrl+E)");
}

// ---------------------------------------------------------------------------
// FUSION PAGE — honest placeholder for the node compositor.
// ---------------------------------------------------------------------------

pub(crate) fn fusion_page(app: &mut VeditApp, ctx: &egui::Context) {
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.add_space(ui.available_height() * 0.30);
            ui.label(RichText::new("◇").size(46.0).color(Palette::ACCENT_DIM));
            ui.add_space(8.0);
            ui.label(RichText::new("Fusion").strong().size(22.0));
            ui.label(
                RichText::new("Node-based compositing is on the VEDIT roadmap.")
                    .color(Palette::TEXT_DIM),
            );
            ui.label(
                RichText::new(
                    "Layer compositing (blend modes, opacity keyframes, transforms)\nis already available in the Edit page Inspector → Effects.",
                )
                .color(Palette::TEXT_FAINT)
                .size(12.0),
            );
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui
                    .add(
                        Button::new(RichText::new("Go to Edit page").strong().color(Palette::ACCENT_TEXT))
                            .fill(Palette::ACCENT),
                    )
                    .clicked()
                {
                    app.workspace = Workspace::Edit;
                }
                if ui.button("Open Color page").clicked() {
                    app.workspace = Workspace::Color;
                }
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
                ui.label(RichText::new("VEDIT 0.4.0").small().color(Palette::TEXT_FAINT));
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
