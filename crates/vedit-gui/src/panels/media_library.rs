//! Media library panel (left) — searchable asset pool with drag-to-timeline.

use eframe::egui::{
    self, Align2, Color32, CursorIcon, DragAndDrop, FontId, Label, RichText, ScrollArea, Sense,
    SidePanel, TextEdit, Vec2,
};

use vedit_core::media::MediaKind;
use vedit_core::project::TrackKind;
use vedit_core::timeline;

use crate::app::VeditApp;
use crate::theme::{panel_header, Palette};

pub(crate) fn media_library(app: &mut VeditApp, ctx: &egui::Context) {
    SidePanel::left("media_library")
        .min_width(240.0)
        .default_width(300.0)
        .width_range(240.0..=430.0)
        .resizable(true)
        .show(ctx, |ui| {
            panel_header(ui, "Media");

            // Search / filter + import.
            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(&mut app.media_filter)
                        .hint_text("🔍  Search media…")
                        .desired_width(ui.available_width() - 92.0),
                );
                if ui.button("+ Import").on_hover_text("Ctrl+I — or drop files anywhere").clicked() {
                    app.import_media_dialog();
                }
            });
            ui.add_space(2.0);

            if app.locator.is_none() {
                ui.label(
                    RichText::new("ffmpeg missing — cannot import media.")
                        .small()
                        .color(Palette::DANGER),
                );
            }

            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let filter = app.media_filter.to_lowercase();
                let assets: Vec<_> = app
                    .project
                    .assets
                    .clone()
                    .into_iter()
                    .filter(|a| filter.is_empty() || a.name.to_lowercase().contains(&filter))
                    .collect();
                if assets.is_empty() && app.pending_imports == 0 {
                    ui.add_space(12.0);
                    let msg = if filter.is_empty() {
                        "No media yet.\n\nClick “+ Import” to add video, audio or images.\n\nYou can also drag files straight into this window, then drag assets onto the timeline."
                    } else {
                        "No media matches the search."
                    };
                    ui.label(RichText::new(msg).color(Palette::TEXT_FAINT));
                }

                for asset in assets {
                    let id = asset.id;
                    draw_asset_row(app, ui, &asset);
                    ui.horizontal(|ui| {
                        if ui
                            .small_button("＋ Add to timeline")
                            .on_hover_text("Insert at the playhead on the first matching track")
                            .clicked()
                        {
                            add_asset_to_timeline(app, id);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .small_button("✕")
                                .on_hover_text("Remove asset (and its clips)")
                                .clicked()
                            {
                                let removed = app.project.remove_asset(id);
                                app.thumbs.remove(&id);
                                if let Some((_, cid)) = app.selected {
                                    // selection may reference a removed clip
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
                                    "Removed {} ({removed} clip{})",
                                    asset.name,
                                    if removed == 1 { "" } else { "s" }
                                ));
                                app.request_preview_frame();
                            }
                        });
                    });
                    ui.add_space(3.0);
                    ui.separator();
                }

                if app.pending_imports > 0 {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0));
                        ui.label(
                            RichText::new(format!("Importing… {} file(s)", app.pending_imports))
                                .small()
                                .color(Palette::TEXT_DIM),
                        );
                    });
                }
            });
        });
}

fn kind_color(kind: MediaKind) -> Color32 {
    match kind {
        MediaKind::Video => Palette::CLIP_VIDEO_BAR,
        MediaKind::Audio => Palette::CLIP_AUDIO_BAR,
        MediaKind::Image => Palette::CLIP_IMAGE_BAR,
    }
}

fn draw_asset_row(app: &mut VeditApp, ui: &mut egui::Ui, asset: &vedit_core::media::MediaAsset) {
    let id = asset.id;
    ui.horizontal(|ui| {
        // Thumbnail
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(78.0, 44.0), Sense::click_and_drag());
        let p = ui.painter();
        p.rect_filled(rect, 4.0, Palette::SURFACE);
        match app.thumbs.get(&id) {
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

        // Drag source: drop onto the timeline to insert.
        if resp.drag_started() {
            DragAndDrop::set_payload(ui.ctx(), id);
        }
        let resp = if resp.hovered() {
            resp.on_hover_cursor(CursorIcon::Grab)
        } else {
            resp
        };
        let resp = if resp.dragged() {
            resp.on_hover_cursor(CursorIcon::Grabbing)
        } else {
            resp
        };
        if resp.double_clicked() {
            add_asset_to_timeline(app, id);
        }
        resp.on_hover_text(format!(
            "Drag onto the timeline to insert\n{}",
            asset.path.display()
        ));

        // Info
        ui.vertical(|ui| {
            ui.add(
                Label::new(RichText::new(&asset.name).size(12.0).color(Palette::TEXT)).truncate(),
            );
            ui.label(
                RichText::new(asset.summary())
                    .size(10.0)
                    .color(Palette::TEXT_DIM),
            );
            ui.label(
                RichText::new(asset.kind.label())
                    .size(9.0)
                    .strong()
                    .color(kind_color(asset.kind)),
            );
        });
    });
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
