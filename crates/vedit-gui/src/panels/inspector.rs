//! Inspector panel (right): selected clip properties or project settings.

use eframe::egui::{self, Checkbox, DragValue, Grid, RichText, TextEdit};

use vedit_core::media::format_timecode;
use vedit_core::project::Clip;
use vedit_core::timeline;

use crate::app::{FPS_PRESETS, RES_PRESETS, PreviewQuality, VeditApp};
use crate::theme::Palette;

pub(crate) fn inspector(app: &mut VeditApp, ctx: &egui::Context) {
    egui::SidePanel::right("inspector")
        .min_width(250.0)
        .default_width(300.0)
        .resizable(true)
        .show(ctx, |ui| {
            ui.heading("Inspector");
            ui.separator();

            let sel = app.selected;
            if let Some((track_id, clip_id)) = sel {
                let clip = app.project.find_clip(track_id, clip_id).cloned();
                match clip {
                    Some(clip) => {
                        let locked = app.project.track(track_id).map(|t| t.locked).unwrap_or(false);
                        clip_inspector(app, ui, track_id, clip, locked);
                    }
                    None => {
                        app.selected = None;
                        project_inspector(app, ui);
                    }
                }
            } else {
                project_inspector(app, ui);
            }
        });
}

fn clip_inspector(app: &mut VeditApp, ui: &mut egui::Ui, track_id: u64, clip: Clip, locked: bool) {
    let asset = app.project.asset(clip.asset_id).cloned();
    let track_name = app.project.track(track_id).map(|t| t.name.clone()).unwrap_or_default();
    let is_image = asset.as_ref().map(|a| a.kind == vedit_core::media::MediaKind::Image).unwrap_or(false);

    ui.label(
        RichText::new(
            asset
                .as_ref()
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "(missing asset)".into()),
        )
        .strong(),
    );
    ui.label(
        RichText::new(format!(
            "{track_name} · {} · source {} → {}",
            track_kind_label(app, track_id),
            vedit_core::media::format_duration(clip.in_point),
            vedit_core::media::format_duration(clip.source_end()),
        ))
        .small()
        .color(Palette::TEXT_DIM),
    );
    ui.add_space(4.0);

    if locked {
        ui.label(RichText::new("Track is locked — edits disabled.").small().color(Palette::DANGER));
        ui.add_space(4.0);
    }

    let mut c = clip.clone();
    let mut changed = false;

    Grid::new("clip_grid")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label("Start");
            changed |= ui
                .add_enabled(!locked, DragValue::new(&mut c.start).speed(0.05).suffix(" s"))
                .changed();
            ui.end_row();

            ui.label("Duration");
            changed |= ui
                .add_enabled(
                    !locked,
                    DragValue::new(&mut c.duration).speed(0.05).range(0.05..=3600.0).suffix(" s"),
                )
                .changed();
            ui.end_row();

            if !is_image {
                ui.label("Speed");
                changed |= ui
                    .add_enabled(
                        !locked,
                        DragValue::new(&mut c.speed).speed(0.05).range(0.25..=4.0).suffix("×"),
                    )
                    .changed();
                ui.end_row();
            }

            ui.label("Opacity");
            changed |= ui
                .add_enabled(
                    !locked,
                    DragValue::new(&mut c.opacity).speed(0.02).range(0.0..=1.0),
                )
                .changed();
            ui.end_row();

            if asset.as_ref().map(|a| a.has_audio()).unwrap_or(false) {
                ui.label("Volume");
                changed |= ui
                    .add_enabled(!locked, DragValue::new(&mut c.volume).speed(0.02).range(0.0..=2.0))
                    .changed();
                ui.end_row();

                ui.label("Mute clip");
                changed |= ui
                    .add_enabled(!locked, Checkbox::new(&mut c.muted, ""))
                    .changed();
                ui.end_row();
            }
        });

    if changed && !locked {
        apply_clip(app, track_id, c);
    }

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button("Split at playhead").on_hover_text("S").clicked() {
            app.split_action();
        }
        if ui
            .button("Delete")
            .on_hover_text("Del")
            .clicked()
        {
            app.delete_selected();
        }
    });

    ui.add_space(8.0);
    ui.separator();
    ui.label(
        RichText::new(format!(
            "Playhead inside clip: {} s from clip start",
            format_timecode((app.playhead - clip.start).max(0.0), app.project.fps)
        ))
        .small()
        .color(Palette::TEXT_DIM),
    );
}

fn track_kind_label(app: &VeditApp, track_id: u64) -> &'static str {
    app.project
        .track(track_id)
        .map(|t| t.kind.label())
        .unwrap_or("?")
}

fn apply_clip(app: &mut VeditApp, track_id: u64, c: Clip) {
    let dur = c.duration.max(timeline::MIN_CLIP_DUR);
    let clamped_start = {
        let Some(track) = app.project.track(track_id) else { return };
        timeline::clamp_placement(track, Some(c.id), c.start.max(0.0), dur)
    };
    if let Some(track) = app.project.track_mut(track_id) {
        if let Some(dst) = track.find_clip_mut(c.id) {
            dst.start = clamped_start;
            dst.duration = dur;
            dst.speed = c.speed.clamp(0.25, 4.0);
            dst.volume = c.volume.clamp(0.0, 2.0);
            dst.opacity = c.opacity.clamp(0.0, 1.0);
            dst.muted = c.muted;
        }
        track
            .clips
            .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    }
    app.mark_dirty();
    app.request_preview_frame();
}

fn project_inspector(app: &mut VeditApp, ui: &mut egui::Ui) {
    ui.label(RichText::new("Project Settings").strong());
    ui.add_space(4.0);

    Grid::new("proj_grid")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label("Name");
            if ui
                .add(TextEdit::singleline(&mut app.project.name).desired_width(160.0))
                .changed()
            {
                app.mark_dirty();
            }
            ui.end_row();

            ui.label("Width");
            if ui
                .add(DragValue::new(&mut app.project.width).speed(2.0).range(16..=7680))
                .changed()
            {
                app.mark_dirty();
                app.request_preview_frame();
            }
            ui.end_row();

            ui.label("Height");
            if ui
                .add(DragValue::new(&mut app.project.height).speed(2.0).range(16..=4320))
                .changed()
            {
                app.mark_dirty();
                app.request_preview_frame();
            }
            ui.end_row();

            ui.label("Frame rate");
            if ui
                .add(DragValue::new(&mut app.project.fps).speed(0.05).range(1.0..=240.0).suffix(" fps"))
                .changed()
            {
                app.mark_dirty();
                app.request_preview_frame();
            }
            ui.end_row();
        });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Presets:").small().color(Palette::TEXT_DIM));
        egui::ComboBox::from_id_salt("proj_res")
            .selected_text(RES_PRESETS[0].0)
            .width(150.0)
            .show_ui(ui, |ui| {
                for (label, w, h) in RES_PRESETS {
                    if ui.selectable_label(false, *label).clicked() {
                        app.project.width = *w;
                        app.project.height = *h;
                        app.mark_dirty();
                        app.request_preview_frame();
                    }
                }
            });
        egui::ComboBox::from_id_salt("proj_fps")
            .selected_text(format!("{:.3}", app.project.fps))
            .width(70.0)
            .show_ui(ui, |ui| {
                for f in FPS_PRESETS {
                    if ui.selectable_label(false, format!("{f:.3}")).clicked() {
                        app.project.fps = *f;
                        app.mark_dirty();
                        app.request_preview_frame();
                    }
                }
            });
    });

    ui.add_space(8.0);
    ui.separator();

    Grid::new("stats_grid")
        .num_columns(2)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            ui.label(RichText::new("Duration").color(Palette::TEXT_DIM));
            ui.label(format_timecode(app.project.duration(), app.project.fps));
            ui.end_row();
            ui.label(RichText::new("Clips").color(Palette::TEXT_DIM));
            ui.label(app.project.clip_count().to_string());
            ui.end_row();
            ui.label(RichText::new("Tracks").color(Palette::TEXT_DIM));
            ui.label(app.project.tracks.len().to_string());
            ui.end_row();
            ui.label(RichText::new("Assets").color(Palette::TEXT_DIM));
            ui.label(app.project.assets.len().to_string());
            ui.end_row();
        });

    ui.add_space(8.0);
    ui.label(RichText::new("Preview quality").color(Palette::TEXT_DIM));
    let mut q = app.preview_quality;
    egui::ComboBox::from_id_salt("ins_quality")
        .selected_text(q.label())
        .width(120.0)
        .show_ui(ui, |ui| {
            for opt in PreviewQuality::ALL {
                ui.selectable_value(&mut q, opt, opt.label());
            }
        });
    if q != app.preview_quality {
        app.preview_quality = q;
        app.seek_inflight = false;
        app.request_preview_frame();
    }

    ui.add_space(12.0);
    ui.separator();
    ui.label(RichText::new("Shortcuts").strong().small());
    for line in [
        "Space  play / pause",
        "S      split at playhead",
        "Del    delete selected clip",
        "← →    step 1 frame (Shift = 10)",
        "Home / End  jump to start / end",
        "Ctrl+S save · Ctrl+E export · Ctrl+O open · Ctrl+I import",
        "Wheel  scroll timeline · Ctrl+Wheel zoom",
    ] {
        ui.label(RichText::new(line).small().color(Palette::TEXT_DIM));
    }

    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
        ui.add_space(6.0);
        ui.label(
            RichText::new("Timeline tip: drag clips to move, drag edges to trim, drop media from the library to insert.")
                .small()
                .color(Palette::TEXT_DIM),
        );
    });
}
