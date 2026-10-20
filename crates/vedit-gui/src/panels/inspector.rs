//! Inspector panel (right): selected clip properties or project settings.
//! v0.2.0: text/title editing, fades & transitions, keyframes, color grade.

use eframe::egui::{self, Checkbox, Color32, DragValue, Grid, RichText, TextEdit};

use vedit_core::media::format_timecode;
use vedit_core::project::{Clip, ColorGrade, Transition, TransitionKind};
use vedit_core::timeline;

use crate::app::{FPS_PRESETS, RES_PRESETS, PreviewQuality, VeditApp};
use crate::theme::Palette;

pub(crate) fn inspector(app: &mut VeditApp, ctx: &egui::Context) {
    egui::SidePanel::right("inspector")
        .min_width(250.0)
        .default_width(310.0)
        .resizable(true)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
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
        });
}

fn clip_inspector(app: &mut VeditApp, ui: &mut egui::Ui, track_id: u64, clip: Clip, locked: bool) {
    let asset = app.project.asset(clip.asset_id).cloned();
    let track_name = app.project.track(track_id).map(|t| t.name.clone()).unwrap_or_default();
    let is_text = clip.is_text();
    let is_image = asset
        .as_ref()
        .map(|a| a.kind == vedit_core::media::MediaKind::Image)
        .unwrap_or(false);

    if is_text {
        let title = clip
            .text
            .as_ref()
            .map(|t| t.content.lines().next().unwrap_or("Title").to_string())
            .unwrap_or_else(|| "Title".into());
        ui.label(RichText::new(title).strong());
        ui.label(
            RichText::new(format!("{track_name} · TEXT · {}", track_kind_label(app, track_id)))
                .small()
                .color(Palette::TEXT_DIM),
        );
    } else {
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
    }

    // Linked A/V + source audio info.
    let asset_has_audio = asset.as_ref().map(|a| a.has_audio()).unwrap_or(false);
    if let Some((lt, _lc)) = app.project.find_linked(track_id, clip.id) {
        let lname = app.project.track(lt).map(|t| t.name.clone()).unwrap_or_default();
        ui.label(
            RichText::new(format!("Linked A/V pair — audio half on {lname}"))
                .small()
                .color(Palette::OK),
        );
    }
    if !is_text && asset.is_some() && !is_image {
        let audio_line = match asset.as_ref() {
            Some(a) if a.has_audio() => format!(
                "Source audio: {} · {} Hz",
                a.info.codec_audio.as_deref().unwrap_or("?"),
                a.info.audio_sample_rate.unwrap_or(0)
            ),
            _ => "Source audio: none".to_string(),
        };
        ui.label(
            RichText::new(audio_line)
                .small()
                .color(if asset_has_audio { Palette::TEXT_DIM } else { Palette::DANGER }),
        );
    }
    ui.add_space(4.0);

    if locked {
        ui.label(RichText::new("Track is locked — edits disabled.").small().color(Palette::DANGER));
        ui.add_space(4.0);
    }

    // ---- text editing ------------------------------------------------------
    if is_text {
        text_section(app, ui, track_id, &clip, locked);
        ui.add_space(6.0);
    }

    // ---- base properties ---------------------------------------------------
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

            if !is_image && !is_text {
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

            if asset_has_audio && !is_text {
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
        apply_clip(app, track_id, c.clone(), "prop");
    }

    // ---- fades & transitions ------------------------------------------------
    ui.add_space(6.0);
    fades_transition_section(app, ui, track_id, &c, locked);

    // ---- keyframes -----------------------------------------------------------
    ui.add_space(6.0);
    keyframes_section(app, ui, track_id, &c, locked);

    // ---- color grading ---------------------------------------------------------
    ui.add_space(6.0);
    color_section(app, ui, track_id, &c, locked);

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button("Split at playhead").on_hover_text("S").clicked() {
            app.split_action();
        }
        if ui.button("Delete").on_hover_text("Del").clicked() {
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

// ---------------------------------------------------------------------------

fn text_section(app: &mut VeditApp, ui: &mut egui::Ui, track_id: u64, clip: &Clip, locked: bool) {
    ui.label(RichText::new("Text").strong().small());
    let mut text = clip.text.clone().unwrap_or_default();
    let mut changed = false;
    ui.add_enabled(!locked, TextEdit::multiline(&mut text.content).desired_width(f32::INFINITY).desired_rows(2));
    Grid::new("text_grid")
        .num_columns(2)
        .spacing([10.0, 5.0])
        .show(ui, |ui| {
            ui.label("Size");
            changed |= ui
                .add_enabled(!locked, DragValue::new(&mut text.size).speed(2.0).range(12.0..=480.0).suffix(" px"))
                .changed();
            ui.end_row();

            ui.label("Bold");
            changed |= ui.add_enabled(!locked, Checkbox::new(&mut text.bold, "")).changed();
            ui.end_row();

            ui.label("Align");
            let align_label = if text.align < 0.34 {
                "Left"
            } else if text.align > 0.66 {
                "Right"
            } else {
                "Center"
            };
            ui.horizontal(|ui| {
                if ui.selectable_label(text.align < 0.34, "L").clicked() {
                    text.align = 0.0;
                    changed = true;
                }
                if ui.selectable_label((0.34..=0.66).contains(&text.align), "C").clicked() {
                    text.align = 0.5;
                    changed = true;
                }
                if ui.selectable_label(text.align > 0.66, "R").clicked() {
                    text.align = 1.0;
                    changed = true;
                }
                ui.label(RichText::new(align_label).small().color(Palette::TEXT_DIM));
            });
            ui.end_row();

            ui.label("Pos X");
            changed |= ui
                .add_enabled(!locked, DragValue::new(&mut text.x).speed(0.01).range(0.0..=1.0))
                .changed();
            ui.end_row();

            ui.label("Pos Y");
            changed |= ui
                .add_enabled(!locked, DragValue::new(&mut text.y).speed(0.01).range(0.0..=1.0))
                .changed();
            ui.end_row();
        });

    // Color swatches.
    ui.horizontal(|ui| {
        ui.label("Color");
        let swatches: [[u8; 4]; 8] = [
            [255, 255, 255, 255],
            [24, 24, 26, 255],
            [224, 180, 88, 255],
            [224, 108, 90, 255],
            [127, 176, 138, 255],
            [120, 160, 210, 255],
            [232, 230, 227, 255],
            [255, 140, 200, 255],
        ];
        for sw in swatches {
            let (r, g, b, a) = (sw[0], sw[1], sw[2], sw[3]);
            let _selected = text.color == sw;
            let btn = egui::Button::new(RichText::new("  ").small())
                .fill(Color32::from_rgba_unmultiplied(r, g, b, a))
                .min_size(egui::vec2(18.0, 14.0));
            if ui.add(btn).clicked() && !locked {
                text.color = sw;
                changed = true;
            }
        }
    });

    if changed && !locked {
        let mut c = clip.clone();
        c.text = Some(text);
        apply_clip(app, track_id, c, "text");
    }
}

// ---------------------------------------------------------------------------

fn fades_transition_section(app: &mut VeditApp, ui: &mut egui::Ui, track_id: u64, clip: &Clip, locked: bool) {
    ui.label(RichText::new("Fades & Transition").strong().small());
    let mut c = clip.clone();
    let mut changed = false;

    Grid::new("fade_grid")
        .num_columns(2)
        .spacing([10.0, 5.0])
        .show(ui, |ui| {
            let max_fade = (clip.duration * 0.9).min(timeline::MAX_FADE);
            ui.label("Fade in");
            changed |= ui
                .add_enabled(!locked, DragValue::new(&mut c.fade_in).speed(0.05).range(0.0..=max_fade).suffix(" s"))
                .changed();
            ui.end_row();

            ui.label("Fade out");
            changed |= ui
                .add_enabled(!locked, DragValue::new(&mut c.fade_out).speed(0.05).range(0.0..=max_fade).suffix(" s"))
                .changed();
            ui.end_row();

            ui.label("Transition out");
            ui.horizontal(|ui| {
                let mut has = c.transition_out.is_some();
                if ui.selectable_label(has, "Cross-dissolve").on_hover_text("Dissolve into the next clip on this track").clicked() && !has {
                    c.transition_out = Some(Transition { kind: TransitionKind::CrossDissolve, duration: 0.5 });
                    changed = true;
                    has = true;
                }
                if ui.selectable_label(!has, "None").clicked() && has {
                    c.transition_out = None;
                    changed = true;
                }
            });
            ui.end_row();

            if let Some(tr) = &mut c.transition_out {
                ui.label("Dissolve length");
                changed |= ui
                    .add_enabled(
                        !locked,
                        DragValue::new(&mut tr.duration)
                            .speed(0.05)
                            .range(0.1..=timeline::MAX_TRANSITION)
                            .suffix(" s"),
                    )
                    .changed();
                ui.end_row();
            }
        });

    if changed && !locked {
        apply_clip(app, track_id, c, "fx");
        if let Some(label) = transition_note(app, track_id, clip.id) {
            ui.label(RichText::new(label).small().color(Palette::TEXT_DIM));
        }
    } else if let Some(tr) = &clip.transition_out {
        let d = timeline::effective_overlap(&app.project, track_id, clip);
        ui.label(
            RichText::new(format!(
                "{} · effective overlap {:.2} s (clamped by clip heads/tails)",
                tr.kind.label(),
                d
            ))
            .small()
            .color(Palette::TEXT_DIM),
        );
    }
}

fn transition_note(app: &VeditApp, track_id: u64, clip_id: u64) -> Option<String> {
    let clip = app.project.find_clip(track_id, clip_id)?;
    let tr = clip.transition_out.as_ref()?;
    let d = timeline::effective_overlap(&app.project, track_id, clip);
    if d <= 0.0 {
        Some(format!(
            "{} has no effective overlap — it needs a following clip on {} with head material",
            tr.kind.label(),
            app.project.track(track_id).map(|t| t.name.clone()).unwrap_or_default()
        ))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------

fn keyframes_section(app: &mut VeditApp, ui: &mut egui::Ui, track_id: u64, clip: &Clip, locked: bool) {
    ui.label(RichText::new("Keyframes (opacity · volume)").strong().small());
    let inside = app.playhead >= clip.start && app.playhead <= clip.end();
    let local_t = (app.playhead - clip.start).clamp(0.0, clip.duration);

    ui.horizontal(|ui| {
        if ui
            .add_enabled(!locked && inside, egui::Button::new("◆ Opacity"))
            .on_hover_text("Add/update an opacity keyframe at the playhead")
            .clicked()
        {
            let v = clip.opacity_at(local_t, 0.0);
            let mut c = clip.clone();
            timeline::upsert_keyframe(&mut c, local_t, Some(v), None);
            apply_clip(app, track_id, c, "key");
        }
        if ui
            .add_enabled(!locked && inside, egui::Button::new("◆ Volume"))
            .on_hover_text("Add/update a volume keyframe at the playhead")
            .clicked()
        {
            let v = clip.volume_at(local_t);
            let mut c = clip.clone();
            timeline::upsert_keyframe(&mut c, local_t, None, Some(v));
            apply_clip(app, track_id, c, "key");
        }
        if !clip.keyframes.is_empty()
            && ui
                .add_enabled(!locked && inside, egui::Button::new("✕"))
                .on_hover_text("Remove keyframe at the playhead")
                .clicked()
        {
            let mut c = clip.clone();
            timeline::remove_keyframe_at(&mut c, local_t);
            apply_clip(app, track_id, c, "key");
        }
        if !clip.keyframes.is_empty() && ui.button("Clear").clicked() {
            let mut c = clip.clone();
            c.keyframes.clear();
            apply_clip(app, track_id, c, "key");
        }
    });
    if !inside {
        ui.label(
            RichText::new("Move the playhead inside the clip to add keyframes.")
                .small()
                .color(Palette::TEXT_DIM),
        );
    }

    if !clip.keyframes.is_empty() {
        egui::ScrollArea::vertical().max_height(90.0).show(ui, |ui| {
            Grid::new("kf_grid")
                .num_columns(4)
                .spacing([8.0, 3.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("t").small().color(Palette::TEXT_DIM));
                    ui.label(RichText::new("opacity").small().color(Palette::TEXT_DIM));
                    ui.label(RichText::new("volume").small().color(Palette::TEXT_DIM));
                    ui.end_row();
                    for k in &clip.keyframes {
                        ui.label(RichText::new(format!("{:.2}s", k.t)).small());
                        ui.label(RichText::new(format!("{:.2}", k.opacity)).small());
                        ui.label(RichText::new(format!("{:.2}", k.volume)).small());
                        ui.end_row();
                    }
                });
        });
    } else {
        ui.label(
            RichText::new("No keyframes — opacity/volume are constant (fades still apply).")
                .small()
                .color(Palette::TEXT_DIM),
        );
    }
}

// ---------------------------------------------------------------------------

fn color_section(app: &mut VeditApp, ui: &mut egui::Ui, track_id: u64, clip: &Clip, locked: bool) {
    ui.label(RichText::new("Color").strong().small());
    let mut grade = clip.grade.clone().unwrap_or_default();
    let mut changed = false;
    let mut sliders: Vec<(&str, f64, f64, f64, f64)> = vec![
        ("Exposure", grade.exposure, -2.0, 2.0, 0.01),
        ("Contrast", grade.contrast, -1.0, 1.0, 0.01),
        ("Saturation", grade.saturation, 0.0, 2.0, 0.01),
        ("Temperature", grade.temperature, -1.0, 1.0, 0.01),
        ("Tint", grade.tint, -1.0, 1.0, 0.01),
    ];
    for (name, val, min, max, speed) in sliders.iter_mut() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(*name).small());
            ui.add_space(4.0);
            let mut v = *val;
            if ui
                .add_enabled(
                    !locked,
                    DragValue::new(&mut v)
                        .speed(*speed)
                        .range(*min..=*max)
                        .max_decimals(2),
                )
                .changed()
            {
                *val = v;
                changed = true;
            }
        });
    }
    grade.exposure = sliders[0].1;
    grade.contrast = sliders[1].1;
    grade.saturation = sliders[2].1;
    grade.temperature = sliders[3].1;
    grade.tint = sliders[4].1;

    // LUT loading.
    ui.horizontal(|ui| {
        if ui.add_enabled(!locked, egui::Button::new("Load LUT…")).on_hover_text(".cube 3D LUT").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Choose a .cube LUT")
                .add_filter("3D LUT", &["cube"])
                .pick_file()
            {
                grade.lut_path = Some(path);
                changed = true;
            }
        }
        if grade.lut_path.is_some() && ui.button("Clear LUT").clicked() {
            grade.lut_path = None;
            changed = true;
        }
    });
    if let Some(name) = grade.lut_name() {
        ui.label(RichText::new(format!("LUT: {name}")).small().color(Palette::OK));
    }

    ui.horizontal(|ui| {
        if ui.add_enabled(!clip.grade.is_none(), egui::Button::new("Reset grade")).clicked() {
            let mut c = clip.clone();
            c.grade = None;
            apply_clip(app, track_id, c, "grade");
            changed = false;
        }
        if ui.button("Neutral").on_hover_text("Reset sliders to no-op").clicked() {
            grade = ColorGrade {
                lut_path: grade.lut_path.clone(),
                ..Default::default()
            };
            changed = true;
        }
    });

    if changed && !locked {
        let mut c = clip.clone();
        c.grade = if grade.is_neutral() { None } else { Some(grade) };
        apply_clip(app, track_id, c, "grade");
    }
}

// ---------------------------------------------------------------------------

fn track_kind_label(app: &VeditApp, track_id: u64) -> &'static str {
    app.project
        .track(track_id)
        .map(|t| t.kind.label())
        .unwrap_or("?")
}

fn apply_clip(app: &mut VeditApp, track_id: u64, c: Clip, tag: &str) {
    let dur = c.duration.max(timeline::MIN_CLIP_DUR);
    let clamped_start = {
        let Some(track) = app.project.track(track_id) else { return };
        timeline::clamp_placement(track, Some(c.id), c.start.max(0.0), dur)
    };
    app.push_undo(tag);
    if let Some(track) = app.project.track_mut(track_id) {
        if let Some(dst) = track.find_clip_mut(c.id) {
            dst.start = clamped_start;
            dst.duration = dur;
            dst.speed = c.speed.clamp(0.25, 4.0);
            dst.volume = c.volume.clamp(0.0, 2.0);
            dst.opacity = c.opacity.clamp(0.0, 1.0);
            dst.muted = c.muted;
            dst.fade_in = c.fade_in.clamp(0.0, dur * 0.9);
            dst.fade_out = c.fade_out.clamp(0.0, dur * 0.9);
            dst.transition_out = c.transition_out;
            dst.keyframes = c.keyframes.clone();
            dst.text = c.text.clone();
            dst.grade = c.grade.clone();
        }
        track
            .clips
            .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    }

    // Mirror the shared properties onto the linked A/V partner (opacity is
    // video-only and stays untouched on the audio half).
    if let Some((pt, pc)) = app.project.find_linked(track_id, c.id) {
        let p_clamped = match app.project.track(pt) {
            Some(t) => timeline::clamp_placement(t, Some(pc), clamped_start.max(0.0), dur),
            None => clamped_start,
        };
        if let Some(t) = app.project.track_mut(pt) {
            if let Some(dst) = t.find_clip_mut(pc) {
                dst.start = p_clamped;
                dst.duration = dur;
                dst.speed = c.speed.clamp(0.25, 4.0);
                dst.volume = c.volume.clamp(0.0, 2.0);
                dst.muted = c.muted;
                // Fades follow the pair so audio and video dissolve together.
                dst.fade_in = c.fade_in.clamp(0.0, dur * 0.9);
                dst.fade_out = c.fade_out.clamp(0.0, dur * 0.9);
            }
            t.clips
                .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
        }
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

            ui.label("Master volume");
            if ui
                .add(
                    DragValue::new(&mut app.project.master_volume)
                        .speed(0.02)
                        .range(0.0..=2.0)
                        .suffix("×"),
                )
                .changed()
            {
                app.mark_dirty();
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
        "Ctrl+Z undo · Ctrl+Y redo",
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
            RichText::new("Tip: drag clips to move, drag edges to trim, drop media from the library to insert.")
                .small()
                .color(Palette::TEXT_DIM),
        );
    });
}
