//! Media pool panel (left) — DaVinci-style BINS + searchable asset pool
//! with drag-to-timeline and drag-to-bin.
//!
//! * Bin tree: Master + user bins (create / delete, drag assets onto a bin
//!   to move them).
//! * Two layouts: compact LIST rows and a thumbnail GRID with duration
//!   badges (toggled from the action bar).

use eframe::egui::{
    self, Align2, Button, Color32, CursorIcon, DragAndDrop, FontId, Label, Pos2, RichText,
    ScrollArea, Sense, SidePanel, Vec2,
};

use vedit_core::media::MediaKind;
use vedit_core::project::TrackKind;
use vedit_core::timeline;

use crate::app::{MediaView, VeditApp};
use crate::theme::{panel_header, Palette};

pub(crate) fn media_library(app: &mut VeditApp, ctx: &egui::Context) {
    SidePanel::left("media_library")
        .min_width(240.0)
        .default_width(310.0)
        .width_range(240.0..=460.0)
        .resizable(true)
        .show(ctx, |ui| {
            panel_header(ui, "Media Pool");

            // Import row.
            ui.horizontal(|ui| {
                if ui
                    .add(Button::new(RichText::new("＋ Import").size(12.0)).small())
                    .on_hover_text("Ctrl+I — or drop files anywhere")
                    .clicked()
                {
                    app.import_media_dialog();
                }
                if app.pending_imports > 0 {
                    ui.add(egui::Spinner::new().size(12.0));
                    ui.label(
                        RichText::new(format!("{} importing…", app.pending_imports))
                            .small()
                            .color(Palette::TEXT_DIM),
                    );
                }
                if app.locator.is_none() {
                    ui.label(RichText::new("ffmpeg missing").small().color(Palette::DANGER));
                }
            });
            ui.add_space(2.0);

            // ---- BIN TREE ------------------------------------------------
            bins_section(app, ui);
            ui.add_space(4.0);
            ui.separator();

            // ---- ASSETS --------------------------------------------------
            let count = app.project.assets.len();
            let shown = filtered_assets(app).len();
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(match (&app.active_bin, app.media_filter.is_empty()) {
                        (Some(b), true) => format!("{b} · {shown}/{count} clips"),
                        (Some(b), false) => format!("{b} · {shown}/{count} match"),
                        (None, true) => format!("All bins · {count} clips"),
                        (None, false) => format!("All bins · {shown}/{count} match"),
                    })
                    .size(10.0)
                    .color(Palette::TEXT_FAINT),
                );
            });
            ui.add_space(2.0);

            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let assets = filtered_assets(app);
                if assets.is_empty() {
                    ui.add_space(12.0);
                    let msg = if count == 0 {
                        "No media yet.\n\nClick “＋ Import” to add video, audio or images.\n\nYou can also drag files straight into this window, then drag assets onto the timeline."
                    } else {
                        "No media matches the current bin / search."
                    };
                    ui.label(RichText::new(msg).color(Palette::TEXT_FAINT));
                }

                match app.media_view {
                    MediaView::Grid => grid_view(app, ui, &assets),
                    MediaView::List => {
                        for asset in &assets {
                            draw_asset_row(app, ui, asset);
                            ui.horizontal(|ui| {
                                if ui
                                    .small_button("＋ Add to timeline")
                                    .on_hover_text("Insert at the playhead on the first matching track")
                                    .clicked()
                                {
                                    add_asset_to_timeline(app, asset.id);
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    bin_menu(app, ui, asset.id);
                                    if ui
                                        .small_button("✕")
                                        .on_hover_text("Remove asset (and its clips)")
                                        .clicked()
                                    {
                                        remove_asset(app, asset.id, &asset.name);
                                    }
                                });
                            });
                            ui.add_space(3.0);
                            ui.separator();
                        }
                    }
                }
            });
        });
}

// ---------------------------------------------------------------------------
// Bins
// ---------------------------------------------------------------------------

fn bins_section(app: &mut VeditApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("BINS").size(9.5).strong().color(Palette::TEXT_FAINT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(Button::new(RichText::new("＋").size(10.0)).small())
                .on_hover_text("New bin")
                .clicked()
            {
                app.bin_creating = true;
                app.bin_name_edit.clear();
            }
        });
    });

    // New-bin inline editor.
    if app.bin_creating {
        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut app.bin_name_edit)
                    .hint_text("bin name…")
                    .desired_width(ui.available_width() - 58.0)
                    .id_salt("new_bin_name"),
            );
            let mut commit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("OK").clicked() {
                commit = true;
            }
            if ui.button("✕").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                app.bin_creating = false;
            }
            if commit {
                let name = app.bin_name_edit.trim().to_string();
                if app.project.add_bin(&name) {
                    app.active_bin = Some(name);
                    app.mark_dirty();
                    app.notify("Bin created");
                } else {
                    app.notify("Bin exists or the name is invalid");
                }
                app.bin_creating = false;
            }
        });
    }

    let bins = app.project.bin_list();
    for bin in bins {
        let n = app
            .project
            .assets
            .iter()
            .filter(|a| a.bin == bin)
            .count();
        let selected = app.active_bin.as_deref() == Some(bin.as_str());

        let (row, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), Sense::click());
        let p = ui.painter();
        if selected {
            p.rect_filled(row, 4.0, Palette::ACCENT_SOFT);
        } else if resp.hovered() {
            p.rect_filled(row, 4.0, Palette::SURFACE_HOVER);
        }
        // Drag-to-bin target: while an asset is being dragged and we hover
        // this row, highlight it; on release, move the asset.
        let dragging: Option<u64> = DragAndDrop::payload::<u64>(ui.ctx()).map(|a| *a);
        if let Some(asset_id) = dragging {
            let hot = resp.hovered();
            p.rect_stroke(
                row,
                4.0,
                egui::Stroke::new(if hot { 1.6_f32 } else { 1.0_f32 }, Palette::ACCENT),
            );
            if hot && ui.ctx().input(|i| i.pointer.any_released()) {
                if app.project.move_asset_to_bin(asset_id, &bin) {
                    app.mark_dirty();
                    app.notify(format!("Moved to bin “{bin}”"));
                }
                DragAndDrop::clear_payload(ui.ctx());
            }
        }
        p.text(
            Pos2::new(row.left() + 8.0, row.center().y),
            Align2::LEFT_CENTER,
            if n > 0 { "▸" } else { "▫" },
            FontId::proportional(9.0),
            Palette::TEXT_DIM,
        );
        p.text(
            Pos2::new(row.left() + 22.0, row.center().y),
            Align2::LEFT_CENTER,
            format!("▤ {bin}"),
            FontId::proportional(11.5),
            if selected { Palette::ACCENT } else { Palette::TEXT },
        );
        p.text(
            Pos2::new(row.right() - 8.0, row.center().y),
            Align2::RIGHT_CENTER,
            format!("{n}"),
            FontId::monospace(9.5),
            Palette::TEXT_FAINT,
        );
        if resp.clicked() {
            // Toggle: click again to go back to all bins.
            app.active_bin = if selected { None } else { Some(bin.clone()) };
        }
        // Delete user bins via right-click.
        if bin != "Master" {
            resp.context_menu(|ui| {
                if ui.button("Delete bin (assets → Master)").clicked() {
                    app.push_undo("bin");
                    let moved = app.project.remove_bin(&bin);
                    if app.active_bin.as_deref() == Some(bin.as_str()) {
                        app.active_bin = None;
                    }
                    app.mark_dirty();
                    app.notify(format!("Bin deleted ({moved} asset(s) moved to Master)"));
                    ui.close_menu();
                }
            });
        }
    }
}

fn bin_menu(app: &mut VeditApp, ui: &mut egui::Ui, asset_id: u64) {
    let bins = app.project.bin_list();
    let current = app
        .project
        .asset(asset_id)
        .map(|a| a.bin.clone())
        .unwrap_or_default();
    ui.menu_button("▤", move |ui| {
        ui.set_min_width(150.0);
        ui.label(RichText::new("Move to bin").small().color(Palette::TEXT_FAINT));
        for b in &bins {
            let mark = if *b == current { " ●" } else { "" };
            if ui.selectable_label(*b == current, format!("{b}{mark}")).clicked() {
                if app.project.move_asset_to_bin(asset_id, b) {
                    app.mark_dirty();
                }
                ui.close_menu();
            }
        }
    })
    .response
    .on_hover_text("Move this asset to another bin");
}

// ---------------------------------------------------------------------------
// Assets
// ---------------------------------------------------------------------------

fn filtered_assets(app: &VeditApp) -> Vec<vedit_core::media::MediaAsset> {
    let filter = app.media_filter.to_lowercase();
    app.project
        .assets
        .iter()
        .filter(|a| app.active_bin.as_ref().map(|b| &a.bin == b).unwrap_or(true))
        .filter(|a| filter.is_empty() || a.name.to_lowercase().contains(&filter))
        .cloned()
        .collect()
}

fn remove_asset(app: &mut VeditApp, id: u64, name: &str) {
    let removed = app.project.remove_asset(id);
    app.thumbs.remove(&id);
    if let Some((_, cid)) = app.selected {
        if removed > 0 {
            let still = app
                .project
                .tracks
                .iter()
                .any(|t| t.clips.iter().any(|c| c.id == cid));
            if !still {
                app.selected = None;
            }
        }
    }
    app.mark_dirty();
    app.notify(format!(
        "Removed {name} ({removed} clip{})",
        if removed == 1 { "" } else { "s" }
    ));
    app.request_preview_frame();
}

fn kind_color(kind: MediaKind) -> Color32 {
    match kind {
        MediaKind::Video => Palette::CLIP_VIDEO_BAR,
        MediaKind::Audio => Palette::CLIP_AUDIO_BAR,
        MediaKind::Image => Palette::CLIP_IMAGE_BAR,
    }
}

fn thumb_painter(app: &VeditApp, ui: &egui::Ui, rect: egui::Rect, asset: &vedit_core::media::MediaAsset) {
    let p = ui.painter();
    p.rect_filled(rect, 4.0, Palette::SURFACE);
    match app.thumbs.get(&asset.id) {
        Some(tex) => {
            let tw = tex.size()[0] as f32;
            let th = tex.size()[1] as f32;
            if tw > 0.0 && th > 0.0 {
                let scale = (rect.width() / tw).min(rect.height() / th);
                let size = Vec2::new(tw * scale, th * scale);
                let dest = egui::Rect::from_center_size(rect.center(), size);
                p.image(
                    tex.id(),
                    dest,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }
        None => {
            p.text(
                rect.center(),
                Align2::CENTER_CENTER,
                asset.kind.label(),
                FontId::proportional(9.0),
                kind_color(asset.kind),
            );
        }
    }
}

/// LIST row (compact, info-dense).
fn draw_asset_row(app: &mut VeditApp, ui: &mut egui::Ui, asset: &vedit_core::media::MediaAsset) {
    let id = asset.id;
    let selected = app.selected_asset == Some(id);
    ui.horizontal(|ui| {
        // Thumbnail (also the drag source).
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(78.0, 44.0), Sense::click_and_drag());
        thumb_painter(app, ui, rect, asset);
        if selected {
            ui.painter()
                .rect_stroke(rect, 4.0, egui::Stroke::new(1.5_f32, Palette::ACCENT));
        }
        asset_drag_source(app, ui, &resp, id);
        if resp.clicked() {
            app.selected_asset = Some(id);
        }
        if resp.double_clicked() {
            add_asset_to_timeline(app, id);
        }

        // Info
        ui.vertical(|ui| {
            ui.add(
                Label::new(RichText::new(&asset.name).size(12.0).color(if selected { Palette::ACCENT } else { Palette::TEXT })).truncate(),
            );
            ui.label(
                RichText::new(asset.summary())
                    .size(10.0)
                    .color(Palette::TEXT_DIM),
            );
            ui.label(
                RichText::new(format!("{} · {}", asset.kind.label(), asset.bin))
                    .size(9.0)
                    .strong()
                    .color(kind_color(asset.kind)),
            );
        });
    });
}

/// GRID card (DaVinci-like thumbnail tiles with a duration badge).
///
/// Two cards per row, name/actions UNDER the thumbnail. The card width is
/// derived from the PANEL width (captured before entering the grid), never
/// from `available_width` inside the grid — a width that depends on the
/// grid's own column sizing feeds back frame-to-frame and makes the cards
/// creep/drift sideways.
fn grid_view(app: &mut VeditApp, ui: &mut egui::Ui, assets: &[vedit_core::media::MediaAsset]) {
    let panel_w = ui.available_width();
    let card_w = ((panel_w - 10.0) * 0.5).max(120.0); // 2 cards + spacing
    let thumb_h = (card_w * 9.0 / 16.0).clamp(56.0, 130.0);
    egui::Grid::new("media_grid")
        .num_columns(2)
        .min_col_width(card_w)
        .spacing([10.0, 10.0])
        .show(ui, |ui| {
            let mut in_row = 0usize;
            for asset in assets {
                if in_row == 2 {
                    ui.end_row();
                    in_row = 0;
                }
                in_row += 1;
                ui.vertical(|ui| {
                    ui.set_min_width(card_w);
                    ui.set_max_width(card_w);
                    let id = asset.id;
                    let selected = app.selected_asset == Some(id);
                    let (rect, resp) = ui
                        .allocate_exact_size(Vec2::new(card_w, thumb_h), Sense::click_and_drag());
                    thumb_painter(app, ui, rect, asset);
                    if selected {
                        ui.painter().rect_stroke(
                            rect,
                            4.0,
                            egui::Stroke::new(1.8_f32, Palette::ACCENT),
                        );
                    }
                    // Duration badge (bottom-right) + kind badge (top-left).
                    let p = ui.painter();
                    if let Some(d) = asset.duration() {
                        let label = vedit_core::media::format_duration(d);
                        let ts = p.layout_no_wrap(label.clone(), FontId::monospace(9.0), Palette::TEXT);
                        let sz = ts.size() + Vec2::new(8.0, 4.0);
                        let badge = egui::Rect::from_min_size(
                            Pos2::new(rect.right() - sz.x - 3.0, rect.bottom() - sz.y - 3.0),
                            sz,
                        );
                        p.rect_filled(badge, 2.0, Color32::from_rgba_premultiplied(10, 10, 12, 220));
                        p.galley(badge.left_top() + Vec2::new(4.0, 2.0), ts, Palette::TEXT);
                    }
                    let kb = egui::Rect::from_min_size(
                        Pos2::new(rect.left() + 3.0, rect.top() + 3.0),
                        Vec2::new(38.0, 13.0),
                    );
                    p.rect_filled(kb, 2.0, Color32::from_rgba_premultiplied(10, 10, 12, 200));
                    p.text(
                        kb.center(),
                        Align2::CENTER_CENTER,
                        asset.kind.label(),
                        FontId::proportional(7.5),
                        kind_color(asset.kind),
                    );
                    asset_drag_source(app, ui, &resp, id);
                    if resp.clicked() {
                        app.selected_asset = Some(id);
                    }
                    if resp.double_clicked() {
                        add_asset_to_timeline(app, id);
                    }

                    // Name + bin line under the tile.
                    ui.add(
                        Label::new(
                            RichText::new(&asset.name)
                                .size(10.5)
                                .color(if selected { Palette::ACCENT } else { Palette::TEXT }),
                        )
                        .truncate(),
                    );
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(asset.bin.clone())
                                .size(9.0)
                                .color(Palette::TEXT_FAINT),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            bin_menu(app, ui, id);
                            if ui
                                .small_button("＋")
                                .on_hover_text("Add to timeline at the playhead")
                                .clicked()
                            {
                                add_asset_to_timeline(app, id);
                            }
                            if ui
                                .small_button("✕")
                                .on_hover_text("Remove asset (and its clips)")
                                .clicked()
                            {
                                remove_asset(app, id, &asset.name);
                            }
                        });
                    });
                    ui.add_space(2.0);
                });
            }
            if in_row > 0 {
                ui.end_row();
            }
        });
}

/// Shared drag-source wiring: payload = asset id (drag to timeline OR bin).
fn asset_drag_source(_app: &mut VeditApp, ui: &mut egui::Ui, resp: &egui::Response, id: u64) {
    if resp.drag_started() {
        DragAndDrop::set_payload(ui.ctx(), id);
    }
    let mut r = resp.clone();
    if r.hovered() {
        r = r.on_hover_cursor(CursorIcon::Grab);
    }
    if r.dragged() {
        r = r.on_hover_cursor(CursorIcon::Grabbing);
    }
    r.on_hover_text("Drag onto the timeline to insert · drag onto a bin to move");
}

pub(crate) fn add_asset_to_timeline(app: &mut VeditApp, asset_id: u64) {
    let Some(asset) = app.project.asset(asset_id).cloned() else {
        return;
    };
    let kind = match asset.kind {
        MediaKind::Audio => TrackKind::Audio,
        _ => TrackKind::Video,
    };
    let Some(tid) = timeline::find_target_track(&app.project, kind, None) else {
        app.notify(format!("No unlocked {} track available", kind.label()));
        return;
    };
    match timeline::add_asset_clip_auto(&mut app.project, asset_id, tid, app.playhead) {
        Ok(made) => {
            let (vtid, vcid) = made[0];
            let track_name = app.project.track(vtid).map(|t| t.name.clone()).unwrap_or_default();
            app.selected = Some((vtid, vcid));
            app.mark_dirty();
            if made.len() > 1 {
                app.notify(format!(
                    "Added {} to {track_name} (linked A/V — audio on {})",
                    asset.name,
                    app.project.track(made[1].0).map(|t| t.name.clone()).unwrap_or_default()
                ));
            } else {
                app.notify(format!("Added to {track_name} at the playhead"));
            }
            app.request_preview_frame();
        }
        Err(e) => app.notify(format!("Could not add clip: {e}")),
    }
}
