//! Inspector panel (right) — DaVinci-style six-tab properties:
//!
//! * **Video** — Transform (Zoom X/Y · Position · Rotation · Anchor point ·
//!   Flip with keyframe diamonds), Dynamic Zoom toggle, Crop, Composition
//!   (blend mode + opacity), Speed change + retime info.
//! * **Audio** — volume + diamond, clip PAN (v0.4.0), mute, track EQ /
//!   multiband compressor section with a live response curve and level
//!   meters, keyframes.
//! * **Effects** — color grade summary + Color page shortcut, attribute
//!   copy/paste, keyframe list.
//! * **Transition** — fades + cross-dissolve controls.
//! * **Image** — image-clip duration + transform.
//! * **File** — codec/format specs, metadata, audio configuration with a
//!   waveform readout, timecode.
//!
//! Every control writes through `apply_clip`, which mirrors shared
//! properties onto the linked A/V partner.

use eframe::egui::{self, Button, Checkbox, Color32, DragValue, FontId, Grid, Pos2, RichText, Stroke, TextEdit, Vec2, Window};

use vedit_core::media::{format_timecode, MediaKind};
use vedit_core::project::{Clip, Transition, TransitionKind};
use vedit_core::timeline::{self, TransformProp};

use crate::app::{FPS_PRESETS, RES_PRESETS, PreviewQuality, VeditApp};
use crate::panels::grade_panel::{apply_clip, grade_controls};
use crate::panels::widgets;
use crate::theme::{mono, Palette};

pub(crate) fn inspector(app: &mut VeditApp, ctx: &egui::Context) {
    egui::SidePanel::right("inspector")
        .min_width(250.0)
        .default_width(330.0)
        .width_range(250.0..=480.0)
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

// ---------------------------------------------------------------------------
// Tab bar
// ---------------------------------------------------------------------------

const TAB_VIDEO: u8 = 0;
const TAB_AUDIO: u8 = 1;
const TAB_FX: u8 = 2;
const TAB_TRANSITION: u8 = 3;
const TAB_IMAGE: u8 = 4;
const TAB_FILE: u8 = 5;

fn tab_button(ui: &mut egui::Ui, app_tab: &mut u8, this: u8, label: &str, tip: &str) {
    if ui
        .selectable_label(*app_tab == this, RichText::new(label).size(12.5))
        .on_hover_text(tip)
        .clicked()
    {
        *app_tab = this;
    }
}

// ---------------------------------------------------------------------------
// Clip inspector (all tabs)
// ---------------------------------------------------------------------------

fn clip_inspector(app: &mut VeditApp, ui: &mut egui::Ui, track_id: u64, clip: Clip, locked: bool) {
    let asset = app.project.asset(clip.asset_id).cloned();
    let track_name = app.project.track(track_id).map(|t| t.name.clone()).unwrap_or_default();
    let is_text = clip.is_text();
    let is_image = asset
        .as_ref()
        .map(|a| a.kind == MediaKind::Image)
        .unwrap_or(false);

    // ---- header ------------------------------------------------------------
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
                    " · proxy"
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

    // ---- base properties (all tabs) ----------------------------------------
    let local_t = (app.playhead - clip.start).clamp(0.0, clip.duration);
    let inside = app.playhead >= clip.start && app.playhead <= clip.end();

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

    // ---- tab bar -------------------------------------------------------------
    // Stale tab repair: Image only exists for image clips.
    if app.inspector_tab == TAB_IMAGE && !is_image {
        app.inspector_tab = TAB_VIDEO;
    }
    ui.horizontal(|ui| {
        tab_button(ui, &mut app.inspector_tab, TAB_VIDEO, "Video", "Transform, crop, compositing, speed");
        ui.label(RichText::new("|").small().color(Palette::LINE));
        tab_button(ui, &mut app.inspector_tab, TAB_AUDIO, "Audio", "Volume, pan, EQ/compressor, meters");
        ui.label(RichText::new("|").small().color(Palette::LINE));
        tab_button(ui, &mut app.inspector_tab, TAB_FX, "Effects", "Color grade, attributes, keyframes");
        ui.label(RichText::new("|").small().color(Palette::LINE));
        tab_button(ui, &mut app.inspector_tab, TAB_TRANSITION, "Transition", "Fades & cross-dissolve");
        if is_image {
            ui.label(RichText::new("|").small().color(Palette::LINE));
            tab_button(ui, &mut app.inspector_tab, TAB_IMAGE, "Image", "Image clip properties");
        }
        ui.label(RichText::new("|").small().color(Palette::LINE));
        tab_button(ui, &mut app.inspector_tab, TAB_FILE, "File", "Codec specs, metadata, waveform");
    });
    ui.separator();

    match app.inspector_tab {
        TAB_VIDEO => video_tab(app, ui, track_id, &c, locked, is_text, is_image, local_t, inside),
        TAB_AUDIO => audio_tab(app, ui, track_id, &c, locked, asset_has_audio, is_text, local_t, inside),
        TAB_FX => effects_tab(app, ui, track_id, &c, locked),
        TAB_TRANSITION => transition_tab(app, ui, track_id, &c, locked),
        TAB_IMAGE => image_tab(app, ui, track_id, &c, locked, local_t, inside),
        _ => file_tab(app, ui, track_id, &c),
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
    _is_image: bool,
    local_t: f64,
    inside: bool,
) {
    if is_text {
        text_section(app, ui, track_id, clip, locked);
        ui.add_space(6.0);
    }

    transform_section(app, ui, track_id, clip, locked, local_t, inside);
    ui.add_space(6.0);

    // Dynamic Zoom (Resolve-style push-in).
    section_label(ui, "Dynamic Zoom");
    let dz = timeline::has_dynamic_zoom(clip);
    ui.horizontal(|ui| {
        let btn = Button::new(RichText::new(if dz { "◉ Dynamic Zoom ON" } else { "○ Dynamic Zoom" }).size(12.0))
            .fill(if dz { Palette::ACCENT_SOFT } else { Palette::SURFACE });
        if ui
            .add(btn)
            .on_hover_text("Slow automatic push-in (1.00 → 1.12 scale across the clip) — implemented with scale keyframes")
            .clicked()
            && !locked
        {
            app.toggle_dynamic_zoom();
        }
        if dz {
            ui.label(RichText::new("1.00 → 1.12").small().color(Palette::ACCENT));
        }
    });
    ui.add_space(6.0);

    crop_section(app, ui, track_id, clip, locked);
    ui.add_space(6.0);

    blend_section(app, ui, track_id, clip, locked, local_t, inside);
    ui.add_space(6.0);

    // Retime & scaling info (honest: fit-to-frame is always the pipeline).
    section_label(ui, "Retime & Scaling");
    ui.label(
        RichText::new("Scaling: Fit to frame · speed changes retime the source (no pitch correction).")
            .small()
            .color(Palette::TEXT_FAINT),
    );
    let _ = inside;
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
            // Zoom X / Y with a link toggle (Resolve: Zoom X / Zoom Y).
            ui.label("Zoom");
            let link_id = egui::Id::new("xf_link");
            let mut link = ui.memory(|m| m.data.get_temp(link_id).unwrap_or(true));
            let link_resp = ui
                .add(Button::new(RichText::new(if link { "🔗" } else { "⛓" }).size(9.0)).small())
                .on_hover_text("Link zoom X/Y");
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

            ui.label("Anchor");
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

    // Keyframe diamonds for zoom / position / rotation.
    ui.horizontal(|ui| {
        ui.label(RichText::new("Keyframes").small().color(Palette::TEXT_DIM));
        let scale_keyed = inside && timeline::has_transform_key_at(clip, local_t, TransformProp::ScaleX);
        if diamond(ui, scale_keyed, "Zoom keyframe at playhead") && !locked && inside {
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
    section_label(ui, "Composition");
    Grid::new("blend_grid")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("Mode");
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

    // ---- header row: vertical meters on the left ----------------------------
    ui.horizontal(|ui| {
        // Stereo meter pair fed by the clip's track bus.
        let track_index = app.project.tracks.iter().position(|t| t.id == track_id);
        let (level, peak) = track_index.map(|i| app.meter_view(i, false)).unwrap_or((0.0, 0.0));
        let mut hold_l = peak;
        let mut hold_r = peak;
        widgets::meter(ui, level, &mut hold_l, 56.0, 7.0);
        widgets::meter(ui, level, &mut hold_r, 56.0, 7.0);
        ui.label(RichText::new("L\nR").small().color(Palette::TEXT_FAINT));

        ui.vertical(|ui| {
            // Volume + keyframe diamond.
            ui.horizontal(|ui| {
                ui.label("Volume");
                let mut c = clip.clone();
                let mut vol = c.volume;
                let dv = ui
                    .add_enabled(!locked, DragValue::new(&mut vol).speed(0.02).range(0.0..=2.0).max_decimals(2))
                    .on_hover_text("Linear gain (double the dB is ×4) — keyframeable");
                let db = widgets::gain_to_db(vol);
                ui.label(
                    RichText::new(format!("{db:+.1} dB"))
                        .monospace()
                        .size(10.0)
                        .color(if (db - 0.0).abs() < 0.05 { Palette::TEXT_DIM } else { Palette::ACCENT }),
                );
                if dv.changed() && !locked {
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

            // Clip PAN (new in v0.4.0) — slider with a center reset.
            ui.horizontal(|ui| {
                ui.label("Pan");
                let mut c = clip.clone();
                let mut pan = c.pan;
                let changed = ui
                    .add_enabled(!locked, egui::Slider::new(&mut pan, -1.0..=1.0).show_value(false))
                    .on_hover_text("Clip stereo balance: −1 = left · 0 = center · +1 = right")
                    .changed();
                ui.label(
                    mono(
                        if pan.abs() < 0.01 {
                            "C".to_string()
                        } else if pan < 0.0 {
                            format!("L {:.0}", -pan * 100.0)
                        } else {
                            format!("R {:.0}", pan * 100.0)
                        },
                        Palette::TEXT_DIM,
                    )
                    .text(),
                );
                if ui.small_button("C").on_hover_text("Reset pan to center").clicked() && !locked {
                    let mut c2 = clip.clone();
                    c2.pan = 0.0;
                    apply_clip(app, track_id, c2, "pan");
                    return;
                }
                if changed && !locked {
                    c.pan = pan;
                    apply_clip(app, track_id, c, "pan");
                }
            });

            // Mute.
            let mut c = clip.clone();
            let mut changed = false;
            ui.horizontal(|ui| {
                if ui.add_enabled(!locked, Checkbox::new(&mut c.muted, "Mute clip")).changed() {
                    changed = true;
                }
                if changed && !locked {
                    apply_clip(app, track_id, c, "prop");
                }
            });
        });
    });

    ui.add_space(4.0);

    // ---- Track EQ / compressor ----------------------------------------------
    section_label(ui, "Track EQ & Compressor");
    let (eq_active, comp_active, bands_on) = match app.project.track(track_id) {
        Some(t) => (
            t.eq.enabled,
            t.comp.enabled,
            t.eq.bands.iter().filter(|b| b.enabled).count(),
        ),
        None => (false, false, 0),
    };
    // Compact response curve of the track's 6-band EQ.
    if let Some(t) = app.project.track(track_id) {
        draw_eq_curve(ui, &t.eq);
    }
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "EQ {} ({bands_on}/6 bands) · Comp {}",
                if eq_active { "ON" } else { "off" },
                if comp_active { "ON" } else { "off" }
            ))
            .small()
            .color(if eq_active || comp_active { Palette::OK } else { Palette::TEXT_FAINT }),
        );
    });
    ui.horizontal(|ui| {
        if ui.button("Equalizer…").on_hover_text("Open the 6-band parametric EQ for this track").clicked() {
            app.eq_dialog_track = Some(track_id);
        }
        if ui.button("Compressor…").on_hover_text("Open the multiband compressor for this track").clicked() {
            app.comp_dialog_track = Some(track_id);
        }
        if ui.button("Mixer page").on_hover_text("Open the Fairlight mixer console").clicked() {
            app.workspace = crate::app::Workspace::Audio;
        }
    });

    ui.add_space(4.0);
    keyframes_section(app, ui, clip);
}

/// Compact log-frequency EQ response curve (−18…+18 dB).
fn draw_eq_curve(ui: &mut egui::Ui, eq: &vedit_core::dsp::EqState) {
    let w = ui.available_width();
    let h = 64.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, Color32::from_rgb(20, 20, 23));
    p.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, Palette::LINE));

    // 0 dB line + ±12 guides.
    for (db, c) in [(0.0_f32, Palette::LINE), (12.0, Palette::LINE_SOFT), (-12.0, Palette::LINE_SOFT)] {
        let y = rect.center().y - db / 18.0 * (rect.height() * 0.5);
        p.line_segment(
            [Pos2::new(rect.left() + 2.0, y), Pos2::new(rect.right() - 2.0, y)],
            Stroke::new(1.0_f32, c),
        );
    }

    let f_min = 20.0_f64;
    let f_max = 20_000.0_f64;
    let x_of = |f: f64| {
        let t = (f.log10() - f_min.log10()) / (f_max.log10() - f_min.log10());
        rect.left() + 2.0 + t as f32 * (rect.width() - 4.0)
    };
    let y_of = |db: f64| rect.center().y - (db.clamp(-18.0, 18.0) as f32) / 18.0 * (rect.height() * 0.5 - 2.0);

    let steps = 96;
    let mut prev: Option<egui::Pos2> = None;
    for i in 0..=steps {
        let f = 10.0_f64.powf(f_min.log10() + (f_max.log10() - f_min.log10()) * (i as f64 / steps as f64));
        let db = eq.response_db(f, 48_000.0);
        let pt = Pos2::new(x_of(f), y_of(db));
        if let Some(pp) = prev {
            p.line_segment([pp, pt], Stroke::new(1.8_f32, Palette::ACCENT));
        }
        prev = Some(pt);
    }
    // Band frequency markers.
    for b in &eq.bands {
        if !b.enabled {
            continue;
        }
        let x = x_of(b.freq);
        p.line_segment(
            [Pos2::new(x, rect.top() + 2.0), Pos2::new(x, rect.bottom() - 2.0)],
            Stroke::new(1.0_f32, Palette::ACCENT_DIM),
        );
    }
    p.text(
        Pos2::new(rect.left() + 6.0, rect.top() + 3.0),
        egui::Align2::LEFT_TOP,
        "20 Hz – 20 kHz · ±18 dB",
        FontId::monospace(8.5),
        Palette::TEXT_FAINT,
    );
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
                            xf.push('R');
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
// EFFECTS tab
// ---------------------------------------------------------------------------

fn effects_tab(app: &mut VeditApp, ui: &mut egui::Ui, track_id: u64, clip: &Clip, locked: bool) {
    // Color grade summary.
    section_label(ui, "Color Correction");
    match &clip.grade {
        Some(g) => {
            let lut = g
                .lut_path
                .as_ref()
                .map(|p| {
                    p.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "LUT".into())
                })
                .unwrap_or_else(|| "no LUT".into());
            ui.label(
                RichText::new(format!(
                    "Grade active · wheels T{:+.2} S{:+.2} · LUT: {lut}",
                    g.temperature, g.tint
                ))
                .small()
                .color(Palette::OK),
            );
        }
        None => {
            ui.label(
                RichText::new("No grade — the clip is neutral.")
                    .small()
                    .color(Palette::TEXT_FAINT),
            );
        }
    }
    if ui.button("Open Color page").on_hover_text("Wheels, scopes, LUT (Alt+5)").clicked() {
        app.workspace = crate::app::Workspace::Color;
    }
    ui.add_space(4.0);
    // Full grade controls also live here (shared with the Color page).
    grade_controls(app, ui, track_id, clip, locked, 56.0);
    ui.add_space(6.0);

    section_label(ui, "Attributes");
    ui.horizontal(|ui| {
        if ui.button("Copy").on_hover_text("Copy this clip's look + effects (Ctrl+C)").clicked() {
            app.copy_attrs_action();
        }
        if ui.button("Paste").on_hover_text("Paste copied look + effects onto this clip (Ctrl+V)").clicked() {
            app.paste_attrs_action();
        }
    });
    ui.add_space(6.0);
    keyframes_section(app, ui, clip);
    let _ = locked;
}

// ---------------------------------------------------------------------------
// TRANSITION tab
// ---------------------------------------------------------------------------

fn transition_tab(app: &mut VeditApp, ui: &mut egui::Ui, track_id: u64, clip: &Clip, locked: bool) {
    fades_transition_section(app, ui, track_id, clip, locked);
}

// ---------------------------------------------------------------------------
// IMAGE tab
// ---------------------------------------------------------------------------

fn image_tab(
    app: &mut VeditApp,
    ui: &mut egui::Ui,
    track_id: u64,
    clip: &Clip,
    locked: bool,
    local_t: f64,
    inside: bool,
) {
    section_label(ui, "Image Clip");
    Grid::new("img_grid")
        .num_columns(2)
        .spacing([10.0, 5.0])
        .show(ui, |ui| {
            ui.label("Duration");
            ui.label(vedit_core::media::format_duration(clip.duration));
            ui.end_row();
            ui.label("Scaling");
            ui.label(RichText::new("Fit to frame (letterboxed)").small().color(Palette::TEXT_DIM));
            ui.end_row();
        });
    ui.add_space(6.0);
    transform_section(app, ui, track_id, clip, locked, local_t, inside);
    let _ = inside;
}

// ---------------------------------------------------------------------------
// FILE tab
// ---------------------------------------------------------------------------

fn file_tab(app: &mut VeditApp, ui: &mut egui::Ui, _track_id: u64, clip: &Clip) {
    let asset = app.project.asset(clip.asset_id).cloned();

    section_label(ui, "Format");
    Grid::new("file_grid")
        .num_columns(2)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            if let Some(a) = &asset {
                let i = &a.info;
                ui.label(RichText::new("Container").color(Palette::TEXT_DIM));
                ui.label(i.format_name.as_deref().unwrap_or("—"));
                ui.end_row();
                if a.kind != MediaKind::Audio {
                    ui.label(RichText::new("Video codec").color(Palette::TEXT_DIM));
                    ui.label(i.codec_video.as_deref().unwrap_or("—"));
                    ui.end_row();
                    ui.label(RichText::new("Resolution").color(Palette::TEXT_DIM));
                    ui.label(format!("{} × {}", i.width.unwrap_or(0), i.height.unwrap_or(0)));
                    ui.end_row();
                    ui.label(RichText::new("Frame rate").color(Palette::TEXT_DIM));
                    ui.label(match i.fps {
                        Some(f) => format!("{f:.3} fps"),
                        None => "—".into(),
                    });
                    ui.end_row();
                }
                if i.has_audio {
                    ui.label(RichText::new("Audio codec").color(Palette::TEXT_DIM));
                    ui.label(i.codec_audio.as_deref().unwrap_or("—"));
                    ui.end_row();
                    ui.label(RichText::new("Sample rate").color(Palette::TEXT_DIM));
                    ui.label(format!("{} Hz", i.audio_sample_rate.unwrap_or(0)));
                    ui.end_row();
                    ui.label(RichText::new("Channels").color(Palette::TEXT_DIM));
                    let ch = i.audio_channels.unwrap_or(0);
                    ui.label(match ch {
                        1 => "Mono".into(),
                        2 => "Stereo".into(),
                        n => format!("{n} channels"),
                    });
                    ui.end_row();
                }
                ui.label(RichText::new("File size").color(Palette::TEXT_DIM));
                ui.label(match i.size_bytes {
                    Some(b) if b > 1024 * 1024 * 1024 => format!("{:.2} GB", b as f64 / 1073741824.0),
                    Some(b) if b > 1024 * 1024 => format!("{:.1} MB", b as f64 / 1048576.0),
                    Some(b) => format!("{b} KB"),
                    None => "—".into(),
                });
                ui.end_row();
            } else {
                ui.label(RichText::new("Text clip — no media file").color(Palette::TEXT_FAINT));
                ui.end_row();
            }
            if let Some(a) = &asset {
                ui.label(RichText::new("Path").color(Palette::TEXT_DIM));
                ui.label(RichText::new(a.path.display().to_string()).small().color(Palette::TEXT_FAINT));
                ui.end_row();
            }
        });

    ui.add_space(6.0);
    section_label(ui, "Metadata");
    Grid::new("meta_grid")
        .num_columns(2)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            ui.label(RichText::new("Clip name").color(Palette::TEXT_DIM));
            ui.label(
                asset
                    .as_ref()
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| "Title".into()),
            );
            ui.end_row();
            ui.label(RichText::new("Timecode").color(Palette::TEXT_DIM));
            ui.label(mono(format_timecode(clip.start, app.project.fps), Palette::TEXT).text());
            ui.end_row();
            ui.label(RichText::new("Timeline in/out").color(Palette::TEXT_DIM));
            ui.label(format!(
                "{} → {}",
                vedit_core::media::format_duration(clip.start),
                vedit_core::media::format_duration(clip.end())
            ));
            ui.end_row();
            ui.label(RichText::new("Source in/out").color(Palette::TEXT_DIM));
            ui.label(format!(
                "{} → {}",
                vedit_core::media::format_duration(clip.in_point),
                vedit_core::media::format_duration(clip.source_end())
            ));
            ui.end_row();
            ui.label(RichText::new("Speed").color(Palette::TEXT_DIM));
            ui.label(format!("{:.2}×", clip.speed));
            ui.end_row();
            ui.label(RichText::new("Bin").color(Palette::TEXT_DIM));
            ui.label(asset.as_ref().map(|a| a.bin.clone()).unwrap_or_else(|| "—".into()));
            ui.end_row();
            if let Some(a) = &asset {
                let age = std::fs::metadata(&a.path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok());
                if let Some(age) = age {
                    ui.label(RichText::new("Data creazione").color(Palette::TEXT_DIM));
                    ui.label(if age.as_secs() < 3600 {
                        format!("{} min ago", age.as_secs() / 60)
                    } else if age.as_secs() < 86_400 {
                        format!("{} h ago", age.as_secs() / 3600)
                    } else {
                        format!("{} days ago", age.as_secs() / 86_400)
                    });
                    ui.end_row();
                }
            }
        });

    // Audio configuration with a waveform readout.
    if let Some(a) = &asset {
        if a.info.has_audio {
            ui.add_space(6.0);
            section_label(ui, "Configurazione audio");
            if let Some(peaks) = app.waveforms.get(&clip.asset_id) {
                draw_waveform(ui, peaks, a.info.audio_channels.unwrap_or(1));
            } else {
                ui.label(
                    RichText::new("Waveform still decoding…")
                        .small()
                        .color(Palette::TEXT_FAINT),
                );
            }
            let ch = a.info.audio_channels.unwrap_or(0);
            let ch_label = match ch {
                1 => "Mono".to_string(),
                2 => "Sinistra - 1 · Destra - 2 (stereo)".to_string(),
                n => format!("{n} channels"),
            };
            ui.label(
                RichText::new(format!(
                    "{ch_label} · {} Hz · {}",
                    a.info.audio_sample_rate.unwrap_or(0),
                    a.info.codec_audio.as_deref().unwrap_or("?")
                ))
                .small()
                .color(Palette::TEXT_FAINT),
            );
        }
    }

    ui.add_space(6.0);
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(RichText::new("TC").small().color(Palette::TEXT_FAINT));
        ui.label(
            mono(
                format_timecode(app.playhead, app.project.fps),
                Palette::ACCENT,
            )
            .size(13.0),
        );
    });
}

/// Compact waveform bars from the cached asset peaks.
fn draw_waveform(ui: &mut egui::Ui, peaks: &std::sync::Arc<Vec<f32>>, _channels: u32) {
    let w = ui.available_width();
    let h = 46.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, Color32::from_rgb(20, 20, 23));
    p.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, Palette::LINE));
    let mid = rect.center().y;
    let n = peaks.len().max(1);
    let bars = (rect.width() as usize).clamp(16, 400);
    for b in 0..bars {
        let i0 = b * n / bars;
        let i1 = ((b + 1) * n / bars).max(i0 + 1);
        let peak = peaks[i0..i1.min(n)].iter().fold(0.0_f32, |m, v| m.max(*v));
        let bh = (peak.clamp(0.0, 1.0) * rect.height() * 0.48).max(0.5);
        let x = rect.left() + b as f32 * rect.width() / bars as f32;
        p.line_segment(
            [Pos2::new(x, mid - bh), Pos2::new(x, mid + bh)],
            Stroke::new(1.0_f32, Palette::CLIP_AUDIO_BAR),
        );
    }
    p.line_segment(
        [Pos2::new(rect.left() + 2.0, mid), Pos2::new(rect.right() - 2.0, mid)],
        Stroke::new(1.0_f32, Palette::LINE_SOFT),
    );
}

// ---------------------------------------------------------------------------
// Text section (title clips)
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
// Fades & transition (shared with the Transition tab)
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
        "L      loop playback",
        "I / O  mark in / out",
        "A / B  select / blade tool",
        "S      split at playhead",
        "Del    delete selected clip",
        "Ctrl+Z undo · Ctrl+Y redo",
        "← →    step 1 frame (Shift = 10)",
        "Home / End  jump to start / end",
        "Alt+1…6  switch pages",
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

// ---------------------------------------------------------------------------
// Keyframes window (toggled from the action bar)
// ---------------------------------------------------------------------------

/// Floating window listing every keyframe of the selected clip with
/// jump-to-playhead buttons.
pub(crate) fn keyframes_window(app: &mut VeditApp, ctx: &egui::Context) {
    if !app.keyframes_open {
        return;
    }
    let mut open = true;
    Window::new("Keyframes")
        .open(&mut open)
        .default_width(320.0)
        .default_height(220.0)
        .show(ctx, |ui| {
            let Some((track_id, clip_id)) = app.selected else {
                ui.label(
                    RichText::new("Select a clip to see its keyframes.")
                        .color(Palette::TEXT_FAINT),
                );
                return;
            };
            let Some(clip) = app.project.find_clip(track_id, clip_id).cloned() else {
                return;
            };
            let name = app
                .project
                .asset(clip.asset_id)
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "Title".into());
            ui.label(RichText::new(name).strong());
            ui.label(
                RichText::new(format!(
                    "{} keyframe(s) · clip starts at {}",
                    clip.keyframes.len(),
                    format_timecode(clip.start, app.project.fps)
                ))
                .small()
                .color(Palette::TEXT_DIM),
            );
            ui.separator();
            if clip.keyframes.is_empty() {
                ui.label(
                    RichText::new(
                        "No keyframes yet.\n\nUse the ◆ diamonds in the Inspector\n(Video → Transform/Opacity, Audio → Volume)\nto add them at the playhead.",
                    )
                    .color(Palette::TEXT_FAINT),
                );
                return;
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                Grid::new("kfw_grid")
                    .num_columns(3)
                    .spacing([8.0, 3.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("time").small().color(Palette::TEXT_DIM));
                        ui.label(RichText::new("values").small().color(Palette::TEXT_DIM));
                        ui.label("");
                        ui.end_row();
                        for k in &clip.keyframes {
                            let mut vals = Vec::new();
                            if k.opacity != 1.0 || k.volume != 1.0 {
                                vals.push(format!("op {:.2} · vol {:.2}", k.opacity, k.volume));
                            }
                            if let Some(sx) = k.scale_x {
                                vals.push(format!("zoom {sx:.2}"));
                            }
                            if k.pos_x.is_some() || k.pos_y.is_some() {
                                vals.push(format!("pos {:.2}/{:.2}", k.pos_x.unwrap_or(0.0), k.pos_y.unwrap_or(0.0)));
                            }
                            if k.rotation.is_some() {
                                vals.push(format!("rot {:.1}°", k.rotation.unwrap_or(0.0)));
                            }
                            ui.label(mono(format_timecode(clip.start + k.t, app.project.fps), Palette::TEXT).size(11.0));
                            ui.label(RichText::new(vals.join(" · ")).small().color(Palette::TEXT_DIM));
                            if ui.small_button("→").on_hover_text("Jump the playhead here").clicked() {
                                app.seek_to(clip.start + k.t);
                            }
                            ui.end_row();
                        }
                    });
            });
        });
    if !open {
        app.keyframes_open = false;
    }
}
