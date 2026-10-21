//! Inspector panel (right): selected clip properties or project settings.
//!
//! Clip editing is organized in two tabs, like the reference app:
//! * **Video** — Transform (scale / position / rotation / pivot / flip with
//!   keyframe diamonds), Crop, Blending, Filters (opacity), Color (four
//!   luma wheels + presets + LUT), Text (title clips), Fades & Transition.
//! * **Audio** — volume + keyframe diamond, mute, fades, keyframe list.
//!
//! Every control writes through `apply_clip`, which mirrors shared
//! properties onto the linked A/V partner.

use eframe::egui::{self, Button, Checkbox, Color32, DragValue, Grid, RichText, TextEdit};

use vedit_core::media::format_timecode;
use vedit_core::project::{Clip, Transition, TransitionKind};
use vedit_core::timeline::{self, TransformProp};

use crate::app::{FPS_PRESETS, RES_PRESETS, PreviewQuality, VeditApp};
use crate::panels::grade_panel::{apply_clip, grade_controls};
use crate::theme::Palette;

pub(crate) fn inspector(app: &mut VeditApp, ctx: &egui::Context) {
    egui::SidePanel::right("inspector")
        .min_width(250.0)
        .default_width(320.0)
        .width_range(250.0..=470.0)
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
                "{track_name} · {} · source {} → {}{}",
                track_kind_label(app, track_id),
                vedit_core::media::format_duration(clip.in_point),
                vedit_core::media::format_duration(clip.source_end()),
                if asset.as_ref().map(|a| a.has_proxy()).unwrap_or(false) {
                    " · ⚡proxy"
                } else {
                    ""
                }
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

    // ---- tabs: Video | Audio ------------------------------------------------
    let local_t = (app.playhead - clip.start).clamp(0.0, clip.duration);
    let inside = app.playhead >= clip.start && app.playhead <= clip.end();

    ui.horizontal(|ui| {
        let tab = &mut app.inspector_tab;
        if ui.selectable_label(*tab == 0, RichText::new("📼 Video").size(13.0)).clicked() {
            *tab = 0;
        }
        if ui.selectable_label(*tab == 1, RichText::new("🔊 Audio").size(13.0)).clicked() {
            *tab = 1;
        }
    });
    ui.separator();

    // ---- base properties (both tabs) --------------------------------------
    let mut c = clip.clone();
    let mut base_changed = false;
    Grid::new("clip_grid")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label("Start");
            base_changed |= ui
                .add_enabled(!locked, DragValue::new(&mut c.start).speed(0.05).suffix(" s"))
                .changed();
            ui.end_row();

            ui.label("Duration");
            base_changed |= ui
                .add_enabled(
                    !locked,
                    DragValue::new(&mut c.duration).speed(0.05).range(0.05..=3600.0).suffix(" s"),
                )
                .changed();
            ui.end_row();

            if !is_image && !is_text {
                ui.label("Speed");
                base_changed |= ui
                    .add_enabled(
                        !locked,
                        DragValue::new(&mut c.speed).speed(0.05).range(0.25..=4.0).suffix("×"),
                    )
                    .changed();
                ui.end_row();
            }
        });
    if base_changed && !locked {
        apply_clip(app, track_id, c.clone(), "prop");
    }
    let c = if base_changed {
        app.project.find_clip(track_id, clip.id).cloned().unwrap_or(clip.clone())
    } else {
        clip.clone()
    };

    match app.inspector_tab {
        0 => video_tab(app, ui, track_id, &c, locked, is_text, is_image, local_t, inside),
        _ => audio_tab(app, ui, track_id, &c, locked, asset_has_audio, is_text, local_t, inside),
    }

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
// VIDEO tab
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn video_tab(
    app: &mut VeditApp,
    ui: &mut egui::Ui,
    track_id: u64,
    clip: &Clip,
    locked: bool,
    is_text: bool,
    is_image: bool,
    local_t: f64,
    inside: bool,
) {
    // ---- text editing ------------------------------------------------------
    if is_text {
        text_section(app, ui, track_id, clip, locked);
        ui.add_space(6.0);
    }

    // ---- transform ---------------------------------------------------------
    transform_section(app, ui, track_id, clip, locked, local_t, inside);
    ui.add_space(6.0);

    // ---- crop ---------------------------------------------------------------
    crop_section(app, ui, track_id, clip, locked);
    ui.add_space(6.0);

    // ---- blending + opacity --------------------------------------------------
    blend_section(app, ui, track_id, clip, locked, local_t, inside);
    ui.add_space(6.0);

    // ---- color ---------------------------------------------------------------
    section_label(ui, "Color Correction");
    grade_controls(app, ui, track_id, clip, locked, 64.0);
    ui.add_space(6.0);

    // ---- fades & transition ---------------------------------------------------
    fades_transition_section(app, ui, track_id, clip, locked);

    ui.add_space(6.0);
    // ---- attribute clipboard ---------------------------------------------------
    section_label(ui, "Attributes");
    ui.horizontal(|ui| {
        if ui.button("Copy").on_hover_text("Copy this clip's look + effects (Ctrl+C)").clicked() {
            app.copy_attrs_action();
        }
        if ui.button("Paste").on_hover_text("Paste copied look + effects onto this clip (Ctrl+V)").clicked() {
            app.paste_attrs_action();
        }
    });

    let _ = is_image;
}

fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).strong().small());
}

/// Small diamond keyframe toggle. Filled when a keyframe exists at the
/// playhead for `prop`. Click = add / remove.
fn diamond(ui: &mut egui::Ui, active: bool, tooltip: &str) -> bool {
    let btn = Button::new(RichText::new("◆").size(10.0))
        .small()
        .fill(if active { Palette::ACCENT } else { Palette::SURFACE })
        .min_size(egui::vec2(16.0, 15.0));
    ui.add(btn).on_hover_text(tooltip).clicked()
}

fn transform_section(
    app: &mut VeditApp,
    ui: &mut egui::Ui,
    track_id: u64,
    clip: &Clip,
    locked: bool,
    local_t: f64,
    inside: bool,
) {
    section_label(ui, "Transform");
    let mut t = clip.transform;
    let mut changed = false;

    Grid::new("xf_grid")
        .num_columns(3)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            // Scale X / Y with a link toggle.
            ui.label("Scale");
            let link_id = egui::Id::new("xf_link");
            let mut link = ui.memory(|m| m.data.get_temp(link_id).unwrap_or(true));
            let link_resp = ui
                .add(Button::new(RichText::new(if link { "🔗" } else { "⛓" }).size(9.0)).small())
                .on_hover_text("Link scale X/Y");
            if link_resp.clicked() {
                link = !link;
                ui.memory_mut(|m| m.data.insert_temp(link_id, link));
            }
            let mut sx = t.scale_x;
            let mut sy = t.scale_y;
            let sx_c = ui.add_enabled(!locked, DragValue::new(&mut sx).speed(0.01).range(0.05..=8.0).max_decimals(2)).changed();
            let sy_c = ui.add_enabled(!locked, DragValue::new(&mut sy).speed(0.01).range(0.05..=8.0).max_decimals(2)).changed();
            if (sx_c || sy_c) && !locked {
                if link {
                    let v = if sx_c { sx } else { sy };
                    t.scale_x = v;
                    t.scale_y = v;
                } else {
                    if sx_c {
                        t.scale_x = sx;
                    }
                    if sy_c {
                        t.scale_y = sy;
                    }
                }
                changed = true;
            }
            ui.end_row();

            ui.label("Position");
            let mut px = t.pos_x;
            let mut py = t.pos_y;
            let px_c = ui.add_enabled(!locked, DragValue::new(&mut px).speed(0.005).range(-1.0..=1.0).max_decimals(3)).changed();
            ui.label(RichText::new("X").small().color(Palette::TEXT_DIM));
            let py_c = ui.add_enabled(!locked, DragValue::new(&mut py).speed(0.005).range(-1.0..=1.0).max_decimals(3)).changed();
            ui.label(RichText::new("Y").small().color(Palette::TEXT_DIM));
            if (px_c || py_c) && !locked {
                t.pos_x = px;
                t.pos_y = py;
                changed = true;
            }
            ui.end_row();

            ui.label("Rotation");
            let mut rot = t.rotation;
            if ui.add_enabled(!locked, DragValue::new(&mut rot).speed(0.5).range(-180.0..=180.0).suffix("°").max_decimals(1)).changed() {
                t.rotation = rot;
                changed = true;
            }
            ui.end_row();

            ui.label("Pivot");
            let mut pvx = t.pivot_x;
            let mut pvy = t.pivot_y;
            let pxc = ui.add_enabled(!locked, DragValue::new(&mut pvx).speed(0.01).range(0.0..=1.0).max_decimals(2)).changed();
            ui.label(RichText::new("X").small().color(Palette::TEXT_DIM));
            let pyc = ui.add_enabled(!locked, DragValue::new(&mut pvy).speed(0.01).range(0.0..=1.0).max_decimals(2)).changed();
            ui.label(RichText::new("Y").small().color(Palette::TEXT_DIM));
            if (pxc || pyc) && !locked {
                t.pivot_x = pvx;
                t.pivot_y = pvy;
                changed = true;
            }
            ui.end_row();

            ui.label("Flip");
            ui.horizontal(|ui| {
                if ui.selectable_label(t.flip_h, "H").on_hover_text("Flip horizontally").clicked() && !locked {
                    t.flip_h = !t.flip_h;
                    changed = true;
                }
                if ui.selectable_label(t.flip_v, "V").on_hover_text("Flip vertically").clicked() && !locked {
                    t.flip_v = !t.flip_v;
                    changed = true;
                }
            });
            ui.end_row();
        });

    // Keyframe diamonds for scale / position / rotation.
    ui.horizontal(|ui| {
        ui.label(RichText::new("Keyframes").small().color(Palette::TEXT_DIM));
        let scale_keyed = inside && timeline::has_transform_key_at(clip, local_t, TransformProp::ScaleX);
        if diamond(ui, scale_keyed, "Scale keyframe at playhead") && !locked && inside {
            toggle_transform_key(app, track_id, clip, local_t, TransformProp::ScaleX, clip.scale_x_at(local_t));
            return;
        }
        let pos_keyed = inside
            && (timeline::has_transform_key_at(clip, local_t, TransformProp::PosX)
                || timeline::has_transform_key_at(clip, local_t, TransformProp::PosY));
        if diamond(ui, pos_keyed, "Position keyframes at playhead") && !locked && inside {
            toggle_transform_key(app, track_id, clip, local_t, TransformProp::PosX, clip.pos_x_at(local_t));
            toggle_transform_key(app, track_id, clip, local_t, TransformProp::PosY, clip.pos_y_at(local_t));
            return;
        }
        let rot_keyed = inside && timeline::has_transform_key_at(clip, local_t, TransformProp::Rotation);
        if diamond(ui, rot_keyed, "Rotation keyframe at playhead") && !locked && inside {
            toggle_transform_key(app, track_id, clip, local_t, TransformProp::Rotation, clip.rotation_at(local_t));
            return;
        }
        if clip.transform_keyframed() && ui.small_button("Clear").on_hover_text("Remove ALL transform keyframes").clicked() && !locked {
            let mut c = clip.clone();
            c.keyframes.clear();
            apply_clip(app, track_id, c, "key");
        }
    });

    if changed && !locked {
        let mut c = clip.clone();
        c.transform = t;
        apply_clip(app, track_id, c, "xf");
    }
}

fn toggle_transform_key(app: &mut VeditApp, track_id: u64, clip: &Clip, local_t: f64, prop: TransformProp, value: f64) {
    let mut c = clip.clone();
    if timeline::has_transform_key_at(&c, local_t, prop) {
        timeline::remove_transform_key_at(&mut c, local_t, prop);
        app.notify("Keyframe removed");
    } else {
        timeline::upsert_transform_keyframe(&mut c, local_t, prop, value);
        app.notify("Keyframe added");
    }
    apply_clip(app, track_id, c, "key");
}

fn crop_section(app: &mut VeditApp, ui: &mut egui::Ui, track_id: u64, clip: &Clip, locked: bool) {
    section_label(ui, "Crop");
    let mut crop = clip.crop;
    let mut changed = false;
    Grid::new("crop_grid")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("Left");
            if ui.add_enabled(!locked, DragValue::new(&mut crop.left).speed(0.005).range(0.0..=0.9).max_decimals(3)).changed() {
                changed = true;
            }
            ui.end_row();
            ui.label("Right");
            if ui.add_enabled(!locked, DragValue::new(&mut crop.right).speed(0.005).range(0.0..=0.9).max_decimals(3)).changed() {
                changed = true;
            }
            ui.end_row();
            ui.label("Top");
            if ui.add_enabled(!locked, DragValue::new(&mut crop.top).speed(0.005).range(0.0..=0.9).max_decimals(3)).changed() {
                changed = true;
            }
            ui.end_row();
            ui.label("Bottom");
            if ui.add_enabled(!locked, DragValue::new(&mut crop.bottom).speed(0.005).range(0.0..=0.9).max_decimals(3)).changed() {
                changed = true;
            }
            ui.end_row();
        });
    if changed && !locked {
        let mut c = clip.clone();
        c.crop = crop;
        apply_clip(app, track_id, c, "crop");
    }
}

fn blend_section(
    app: &mut VeditApp,
    ui: &mut egui::Ui,
    track_id: u64,
    clip: &Clip,
    locked: bool,
    local_t: f64,
    inside: bool,
) {
    section_label(ui, "Filters");
    Grid::new("blend_grid")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("Blend");
            let mut blend = clip.blend;
            egui::ComboBox::from_id_salt("blend_mode")
                .selected_text(blend.label())
                .width(110.0)
                .show_ui(ui, |ui| {
                    for m in vedit_core::project::BlendMode::ALL {
                        ui.selectable_value(&mut blend, m, m.label());
                    }
                });
            if blend != clip.blend && !locked {
                let mut c = clip.clone();
                c.blend = blend;
                apply_clip(app, track_id, c, "blend");
            }
            ui.end_row();
        });

    // Opacity with keyframe diamond.
    ui.horizontal(|ui| {
        ui.label("Opacity");
        let mut c = clip.clone();
        let mut op = c.opacity;
        if ui.add_enabled(!locked, DragValue::new(&mut op).speed(0.02).range(0.0..=1.0).max_decimals(2)).changed() {
            c.opacity = op;
            apply_clip(app, track_id, c, "prop");
        }
        let op_keyed = inside
            && clip
                .keyframes
                .iter()
                .any(|k| (k.t - local_t).abs() < 1.0 / 240.0);
        if diamond(ui, op_keyed, "Opacity keyframe at playhead") && !locked && inside {
            let mut c = clip.clone();
            if op_keyed {
                timeline::remove_keyframe_at(&mut c, local_t);
                app.notify("Opacity keyframe removed");
            } else {
                let v = clip.opacity_at(local_t, 0.0);
                timeline::upsert_keyframe(&mut c, local_t, Some(v), None);
                app.notify("Opacity keyframe added");
            }
            apply_clip(app, track_id, c, "key");
        }
    });
}

// ---------------------------------------------------------------------------
// AUDIO tab
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn audio_tab(
    app: &mut VeditApp,
    ui: &mut egui::Ui,
    track_id: u64,
    clip: &Clip,
    locked: bool,
    asset_has_audio: bool,
    is_text: bool,
    local_t: f64,
    inside: bool,
) {
    if !asset_has_audio || is_text {
        ui.label(
            RichText::new("This clip carries no audio.")
                .small()
                .color(Palette::TEXT_DIM),
        );
        return;
    }

    // Volume + keyframe diamond.
    ui.horizontal(|ui| {
        ui.label("Volume");
        let mut c = clip.clone();
        let mut vol = c.volume;
        if ui.add_enabled(!locked, DragValue::new(&mut vol).speed(0.02).range(0.0..=2.0).max_decimals(2)).changed() {
            c.volume = vol;
            apply_clip(app, track_id, c, "prop");
        }
        let vol_keyed = inside && clip.keyframes.iter().any(|k| (k.t - local_t).abs() < 1.0 / 240.0);
        if diamond(ui, vol_keyed, "Volume keyframe at playhead") && !locked && inside {
            let mut c = clip.clone();
            if vol_keyed {
                timeline::remove_keyframe_at(&mut c, local_t);
                app.notify("Volume keyframe removed");
            } else {
                let v = clip.volume_at(local_t);
                timeline::upsert_keyframe(&mut c, local_t, None, Some(v));
                app.notify("Volume keyframe added");
            }
            apply_clip(app, track_id, c, "key");
        }
    });

    // Mute.
    let mut c = clip.clone();
    let mut changed = false;
    Grid::new("audio_grid")
        .num_columns(2)
        .spacing([10.0, 5.0])
        .show(ui, |ui| {
            ui.label("Mute clip");
            if ui.add_enabled(!locked, Checkbox::new(&mut c.muted, "")).changed() {
                changed = true;
            }
            ui.end_row();
        });
    if changed && !locked {
        apply_clip(app, track_id, c, "prop");
    }

    ui.add_space(4.0);
    fades_transition_section(app, ui, track_id, clip, locked);

    ui.add_space(4.0);
    keyframes_section(app, ui, clip);
}

fn keyframes_section(app: &mut VeditApp, ui: &mut egui::Ui, clip: &Clip) {
    ui.label(RichText::new("Keyframes (opacity · volume)").strong().small());
    if !clip.keyframes.is_empty() {
        egui::ScrollArea::vertical().max_height(110.0).show(ui, |ui| {
            Grid::new("kf_grid")
                .num_columns(4)
                .spacing([8.0, 3.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("t").small().color(Palette::TEXT_DIM));
                    ui.label(RichText::new("opacity").small().color(Palette::TEXT_DIM));
                    ui.label(RichText::new("volume").small().color(Palette::TEXT_DIM));
                    ui.label(RichText::new("xform").small().color(Palette::TEXT_DIM));
                    ui.end_row();
                    for k in &clip.keyframes {
                        ui.label(RichText::new(format!("{:.2}s", k.t)).small());
                        ui.label(RichText::new(format!("{:.2}", k.opacity)).small());
                        ui.label(RichText::new(format!("{:.2}", k.volume)).small());
                        let mut xf = String::new();
                        if k.scale_x.is_some() {
                            xf.push_str("S ");
                        }
                        if k.pos_x.is_some() || k.pos_y.is_some() {
                            xf.push_str("P ");
                        }
                        if k.rotation.is_some() {
                            xf.push_str("R");
                        }
                        ui.label(RichText::new(xf).small().color(Palette::ACCENT));
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
    let _ = app;
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

fn track_kind_label(app: &VeditApp, track_id: u64) -> &'static str {
    app.project
        .track(track_id)
        .map(|t| t.kind.label())
        .unwrap_or("?")
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
