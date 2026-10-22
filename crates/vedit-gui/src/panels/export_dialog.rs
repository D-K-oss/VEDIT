//! Export dialog: presets, codec/bitrate control, progress + cancel.

use eframe::egui::{
    self, Button, Checkbox, ComboBox, DragValue, Grid, ProgressBar, RichText, TextEdit, Window,
};
use std::path::PathBuf;

use vedit_core::export::{
    self, ExportJob, ExportSettings, FpsChoice, RateMode, ResChoice,
    AUDIO_CODECS, CONTAINERS, PRESETS, VIDEO_CODECS,
};
use vedit_core::media::format_timecode;

use crate::app::VeditApp;
use crate::theme::{mono, Palette};

pub(crate) struct ExportUi {
    pub open: bool,
    pub settings: ExportSettings,
    pub preset_id: &'static str,
    pub job: Option<ExportJob>,
    pub last_progress: f32,
    pub out_time: f64,
    pub done_msg: Option<String>,
    pub err: Option<String>,
    pub out_path_text: String,
}

impl Default for ExportUi {
    fn default() -> Self {
        let mut ui = ExportUi {
            open: false,
            settings: ExportSettings::default(),
            preset_id: "h264",
            job: None,
            last_progress: 0.0,
            out_time: 0.0,
            done_msg: None,
            err: None,
            out_path_text: "export".into(),
        };
        apply_preset(&mut ui.settings, &mut ui.preset_id, "h264");
        ui
    }
}

impl ExportUi {
    pub fn job_running(&self) -> bool {
        self.job.as_ref().map(|j| j.running()).unwrap_or(false)
    }
}

fn apply_preset(settings: &mut ExportSettings, preset_id: &mut &'static str, id: &'static str) {
    if let Some(p) = PRESETS.iter().find(|p| p.id == id) {
        (p.apply)(settings);
        *preset_id = id;
        sync_video_codec(settings);
    }
}

/// Align rate/preset with the selected codec's capabilities.
fn sync_video_codec(settings: &mut ExportSettings) {
    let Some(def) = export::video_codec_def(settings.video.codec) else {
        return;
    };
    if def.lossless {
        settings.video.rate = RateMode::Bitrate { kbps: 0 };
    } else if !def.supports_crf && matches!(settings.video.rate, RateMode::Crf { .. }) {
        settings.video.rate = RateMode::Bitrate {
            kbps: def.default_bitrate_kbps,
        };
    }
    let preset_ok = settings
        .video
        .preset
        .map(|p| def.presets.contains(&p))
        .unwrap_or(false);
    if !preset_ok {
        settings.video.preset = def.presets.first().copied();
    }
}

const RES_OPTIONS: &[(&str, ResChoice)] = &[
    ("Project resolution", ResChoice::Project),
    ("3840 × 2160 (4K)", ResChoice::Fixed(3840, 2160)),
    ("2560 × 1440 (QHD)", ResChoice::Fixed(2560, 1440)),
    ("1920 × 1080 (HD)", ResChoice::Fixed(1920, 1080)),
    ("1280 × 720", ResChoice::Fixed(1280, 720)),
    ("854 × 480", ResChoice::Fixed(854, 480)),
];

const FPS_OPTIONS: &[(&str, FpsChoice)] = &[
    ("Project fps", FpsChoice::Project),
    ("60", FpsChoice::Fixed(60.0)),
    ("50", FpsChoice::Fixed(50.0)),
    ("30", FpsChoice::Fixed(30.0)),
    ("25", FpsChoice::Fixed(25.0)),
    ("24", FpsChoice::Fixed(24.0)),
];

// ---------------------------------------------------------------------------

pub(crate) fn export_dialog(app: &mut VeditApp, ctx: &egui::Context) {
    if !app.export.open {
        return;
    }
    let mut open = true;

    // Snapshot job info (avoid borrows across UI closures).
    let running = app.export.job_running();
    let done = app.export.done_msg.clone();
    let err = app.export.err.clone();
    let pct = app.export.last_progress;
    let out_time = app.export.out_time;
    let (total, elapsed, out_path) = match &app.export.job {
        Some(j) => (
            j.total_duration,
            j.started.elapsed().as_secs_f64(),
            Some(j.output.clone()),
        ),
        None => (0.0, 0.0, None),
    };

    Window::new("Export")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .min_width(560.0)
        .show(ctx, |ui| {
            if app.export.job.is_some() {
                render_progress(
                    ui, running, done, err, pct, out_time, total, elapsed, out_path, app,
                );
            } else {
                render_form(ui, app);
            }
        });

    if !open {
        app.export.open = false;
    }
}

#[allow(clippy::too_many_arguments)]
fn render_progress(
    ui: &mut egui::Ui,
    running: bool,
    done: Option<String>,
    err: Option<String>,
    pct: f32,
    out_time: f64,
    total: f64,
    elapsed: f64,
    out_path: Option<PathBuf>,
    app: &mut VeditApp,
) {
    ui.add(
        ProgressBar::new((pct / 100.0).clamp(0.0, 1.0))
            .show_percentage()
            .text(format!(
                "{} / {}",
                format_timecode(out_time, app.project.fps),
                format_timecode(total, app.project.fps)
            )),
    );
    if running {
        ui.label(
            RichText::new(format!("ffmpeg rendering · {:.0}s elapsed", elapsed))
                .small()
                .color(Palette::TEXT_DIM),
        );
        ui.add_space(6.0);
        if ui.button("Cancel render").clicked() {
            if let Some(job) = app.export.job.take() {
                job.cancel();
            }
            app.notify("Export cancelled");
        }
        ui.label(
            RichText::new("The render runs in the background; cancel any time.")
                .small()
                .color(Palette::TEXT_DIM),
        );
        return;
    }

    if let Some(out) = done {
        ui.add_space(6.0);
        ui.label(RichText::new("✓ Export finished").strong().color(Palette::OK));
        ui.label(mono(out, Palette::TEXT));
        ui.horizontal(|ui| {
            if let Some(p) = out_path.clone() {
                if ui.button("Open folder").clicked() {
                    if let Some(folder) = p.parent().map(|x| x.to_path_buf()) {
                        let _ = open::that(folder);
                    }
                }
            }
            if ui.button("Close").clicked() {
                app.export.job = None;
                app.export.open = false;
            }
        });
    } else if let Some(e) = err {
        ui.add_space(6.0);
        ui.label(RichText::new("Export failed").strong().color(Palette::DANGER));
        egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
            ui.label(mono(e, Palette::DANGER));
        });
        ui.horizontal(|ui| {
            if ui.button("Close").clicked() {
                app.export.job = None;
            }
        });
    }
}

fn render_form(ui: &mut egui::Ui, app: &mut VeditApp) {
    let audio_only_container = export::is_audio_container(app.export.settings.container);
    let is_gif = app.export.settings.container == "gif";
    let total = app.project.duration();

    // ---- preset ----
    ui.horizontal(|ui| {
        ui.label("Preset");
        let current_label = PRESETS
            .iter()
            .find(|p| p.id == app.export.preset_id)
            .map(|p| p.label)
            .unwrap_or("Custom");
        ComboBox::from_id_salt("preset")
            .selected_text(current_label)
            .width(360.0)
            .show_ui(ui, |ui| {
                for p in PRESETS {
                    let id = p.id;
                    if ui
                        .selectable_label(app.export.preset_id == id, p.label)
                        .clicked()
                    {
                        apply_preset(&mut app.export.settings, &mut app.export.preset_id, id);
                    }
                }
            });
    });

    // ---- container ----
    ui.horizontal(|ui| {
        ui.label("Container");
        ComboBox::from_id_salt("container")
            .selected_text(format!(".{}", app.export.settings.container))
            .width(90.0)
            .show_ui(ui, |ui| {
                for c in CONTAINERS {
                    if ui
                        .selectable_label(app.export.settings.container == *c, format!(".{c}"))
                        .clicked()
                    {
                        app.export.settings.container = c;
                        if export::is_audio_container(c) {
                            app.export.settings.video.enabled = false;
                            app.export.settings.audio.enabled = true;
                        } else if *c == "gif" {
                            app.export.settings.video.enabled = true;
                            app.export.settings.audio.enabled = false;
                        } else {
                            app.export.settings.video.enabled = true;
                        }
                    }
                }
            });
        let hint = if audio_only_container {
            "audio-only container"
        } else if is_gif {
            "animation · video only"
        } else {
            ""
        };
        ui.label(RichText::new(hint).small().color(Palette::TEXT_DIM));
    });

    ui.separator();

    // ---- video ----
    if !audio_only_container {
        let mut v_enabled = app.export.settings.video.enabled;
        if ui.add(Checkbox::new(&mut v_enabled, "Video")).changed() {
            app.export.settings.video.enabled = v_enabled;
        }
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            ui.label("Codec");
            let mut codec = app.export.settings.video.codec;
            ComboBox::from_id_salt("vcodec")
                .selected_text(
                    export::video_codec_def(codec)
                        .map(|d| codec_label(app, d))
                        .unwrap_or_else(|| codec.to_string()),
                )
                .width(280.0)
                .show_ui(ui, |ui| {
                    ui.label(RichText::new("Software").small().color(Palette::TEXT_DIM));
                    for d in VIDEO_CODECS.iter().filter(|d| !d.hw) {
                        ui.selectable_value(&mut codec, d.id, codec_label(app, d));
                    }
                    ui.separator();
                    ui.label(
                        RichText::new("Hardware (GPU)")
                            .small()
                            .color(Palette::TEXT_DIM),
                    );
                    for d in VIDEO_CODECS.iter().filter(|d| d.hw) {
                        ui.selectable_value(&mut codec, d.id, codec_label(app, d));
                    }
                });
            if codec != app.export.settings.video.codec {
                app.export.settings.video.codec = codec;
                sync_video_codec(&mut app.export.settings);
            }
        });
        // Availability hint for hardware encoders.
        if let Some(def) = export::video_codec_def(app.export.settings.video.codec) {
            if def.hw {
                let ok = app.hw_encoders.contains(def.id);
                if ok {
                    ui.label(
                        RichText::new("● Hardware encoder detected — GPU acceleration active")
                            .small()
                            .color(Palette::OK),
                    );
                } else {
                    ui.label(
                        RichText::new(
                            "○ Not detected in this ffmpeg build — export will fail unless a compatible GPU encoder exists",
                        )
                        .small()
                        .color(Palette::DANGER),
                    );
                }
            }
        }

        if app.export.settings.video.enabled {
            let def = export::video_codec_def(app.export.settings.video.codec);
            let supports_crf = def.map(|d| d.supports_crf).unwrap_or(false);
            let lossless = def.map(|d| d.lossless).unwrap_or(false);
            let presets: &'static [&'static str] = def.map(|d| d.presets).unwrap_or(&[]);

            Grid::new("vgrid")
                .num_columns(2)
                .spacing([10.0, 5.0])
                .show(ui, |ui| {
                    if supports_crf && !lossless {
                        // Normalize + edit rate mode.
                        let mode_crf = matches!(app.export.settings.video.rate, RateMode::Crf { .. });
                        ui.label("Rate control");
                        ui.horizontal(|ui| {
                            let mut m = mode_crf;
                            ui.radio_value(&mut m, true, "Quality (CRF)");
                            ui.radio_value(&mut m, false, "Bitrate");
                            if m != mode_crf {
                                app.export.settings.video.rate = if m {
                                    RateMode::Crf { crf: 18 }
                                } else {
                                    RateMode::Bitrate {
                                        kbps: def.map(|d| d.default_bitrate_kbps).unwrap_or(8000),
                                    }
                                };
                            }
                        });
                        ui.end_row();

                        match &mut app.export.settings.video.rate {
                            RateMode::Crf { crf } => {
                                ui.label("CRF (lower = better quality)");
                                ui.add(DragValue::new(crf).range(0..=51));
                                ui.end_row();
                            }
                            RateMode::Bitrate { kbps } => {
                                ui.label("Video bitrate");
                                ui.add(
                                    DragValue::new(kbps).speed(500).range(64..=400_000).suffix(" kbps"),
                                );
                                ui.end_row();
                            }
                        }
                    } else if !lossless {
                        if !matches!(app.export.settings.video.rate, RateMode::Bitrate { .. }) {
                            app.export.settings.video.rate = RateMode::Bitrate {
                                kbps: def.map(|d| d.default_bitrate_kbps).unwrap_or(8000),
                            };
                        }
                        if let RateMode::Bitrate { kbps } = &mut app.export.settings.video.rate {
                            ui.label("Video bitrate");
                            ui.add(
                                DragValue::new(kbps).speed(500).range(64..=400_000).suffix(" kbps"),
                            );
                            ui.end_row();
                        }
                    }

                    if !presets.is_empty() {
                        ui.label("Speed preset");
                        let mut preset = app.export.settings.video.preset;
                        ComboBox::from_id_salt("vpreset")
                            .selected_text(preset.unwrap_or("-"))
                            .width(120.0)
                            .show_ui(ui, |ui| {
                                for p in presets {
                                    ui.selectable_value(&mut preset, Some(*p), *p);
                                }
                            });
                        app.export.settings.video.preset = preset;
                        ui.end_row();
                    }
                });

            // Resolution + fps
            Grid::new("rf_grid")
                .num_columns(2)
                .spacing([10.0, 5.0])
                .show(ui, |ui| {
                    ui.label("Resolution");
                    let mut res = app.export.settings.res;
                    ComboBox::from_id_salt("res")
                        .selected_text(
                            RES_OPTIONS
                                .iter()
                                .find(|(_, r)| *r == res)
                                .map(|(l, _)| *l)
                                .unwrap_or("custom"),
                        )
                        .width(190.0)
                        .show_ui(ui, |ui| {
                            for (label, rc) in RES_OPTIONS {
                                ui.selectable_value(&mut res, *rc, *label);
                            }
                        });
                    app.export.settings.res = res;
                    ui.end_row();

                    ui.label("Frame rate");
                    let mut fps = app.export.settings.fps;
                    ComboBox::from_id_salt("fps")
                        .selected_text(
                            FPS_OPTIONS
                                .iter()
                                .find(|(_, f)| *f == fps)
                                .map(|(l, _)| *l)
                                .unwrap_or("custom"),
                        )
                        .width(190.0)
                        .show_ui(ui, |ui| {
                            for (label, fc) in FPS_OPTIONS {
                                ui.selectable_value(&mut fps, *fc, *label);
                            }
                        });
                    app.export.settings.fps = fps;
                    ui.end_row();
                });
        }
        ui.separator();
    }

    // ---- audio ----
    let audio_disabled_by_gif = is_gif;
    let mut a_enabled = app.export.settings.audio.enabled && !audio_disabled_by_gif;
    if ui
        .add_enabled(!audio_disabled_by_gif, Checkbox::new(&mut a_enabled, "Audio"))
        .changed()
    {
        app.export.settings.audio.enabled = a_enabled;
    }
    if app.export.settings.audio.enabled {
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            ui.label("Codec");
            let mut acodec = app.export.settings.audio.codec;
            ComboBox::from_id_salt("acodec")
                .selected_text(
                    export::audio_codec_def(acodec)
                        .map(|d| d.label)
                        .unwrap_or(acodec),
                )
                .width(250.0)
                .show_ui(ui, |ui| {
                    for d in AUDIO_CODECS {
                        ui.selectable_value(&mut acodec, d.id, d.label);
                    }
                });
            app.export.settings.audio.codec = acodec;
        });

        let alossless = export::audio_codec_def(app.export.settings.audio.codec)
            .map(|d| d.lossless)
            .unwrap_or(false);
        Grid::new("agrid")
            .num_columns(2)
            .spacing([10.0, 5.0])
            .show(ui, |ui| {
                if !alossless {
                    ui.label("Audio bitrate");
                    let mut kbps = app.export.settings.audio.bitrate_kbps;
                    ui.add(DragValue::new(&mut kbps).speed(16).range(32..=990).suffix(" kbps"));
                    app.export.settings.audio.bitrate_kbps = kbps;
                    ui.end_row();
                }
                ui.label("Sample rate");
                ui.horizontal(|ui| {
                    for sr in [48_000u32, 44_100] {
                        if ui
                            .selectable_label(app.export.settings.audio.sample_rate == sr, format!("{sr} Hz"))
                            .clicked()
                        {
                            app.export.settings.audio.sample_rate = sr;
                        }
                    }
                });
                ui.end_row();
            });
    }
    ui.separator();

    // ---- extra args ----
    ui.horizontal(|ui| {
        ui.label("Extra args");
        ui.add(
            TextEdit::singleline(&mut app.export.settings.extra_args)
                .hint_text("e.g. -tune film -x264-params keyint=240")
                .desired_width(360.0),
        );
    });

    // ---- output ----
    ui.horizontal(|ui| {
        ui.label("Output");
        ui.add(TextEdit::singleline(&mut app.export.out_path_text).desired_width(360.0));
        if ui.button("Browse…").clicked() {
            let suggested = format!(
                "{}.{}",
                if app.export.out_path_text.is_empty() { "export" } else { app.export.out_path_text.trim() },
                app.export.settings.container
            );
            if let Some(p) = rfd::FileDialog::new()
                .set_title("Choose export destination")
                .set_file_name(suggested)
                .save_file()
            {
                app.export.settings.output = p.clone();
                app.export.out_path_text = p.display().to_string();
            }
        }
    });
    ui.label(
        RichText::new(format!(
            "Final file will be written as: {} (.{}) · timeline {} ",
            display_output(&app.export.settings),
            app.export.settings.container,
            format_timecode(total, app.project.fps)
        ))
        .small()
        .color(Palette::TEXT_DIM),
    );

    // ---- size estimate ----
    let v_kbps = if app.export.settings.video.enabled && !audio_only_container {
        match app.export.settings.video.rate {
            RateMode::Bitrate { kbps } => {
                let lossless = export::video_codec_def(app.export.settings.video.codec)
                    .map(|d| d.lossless)
                    .unwrap_or(false);
                if lossless {
                    0.0
                } else {
                    kbps as f64
                }
            }
            RateMode::Crf { .. } => 0.0,
        }
    } else {
        0.0
    };
    let a_kbps = if app.export.settings.audio.enabled {
        let lossless = export::audio_codec_def(app.export.settings.audio.codec)
            .map(|d| d.lossless)
            .unwrap_or(false);
        if lossless {
            0.0
        } else {
            app.export.settings.audio.bitrate_kbps as f64
        }
    } else {
        0.0
    };
    let est = if v_kbps + a_kbps > 0.0 {
        format!("≈ {:.1} MB", (v_kbps + a_kbps) * total / 8.0 / 1024.0)
    } else {
        "size depends on content".into()
    };
    ui.label(
        RichText::new(format!("Estimated size: {est}"))
            .small()
            .color(Palette::TEXT_DIM),
    );

    // ---- go ----
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui
            .add(
                Button::new(RichText::new("Export").strong().color(Palette::ACCENT_TEXT))
                    .fill(Palette::ACCENT),
            )
            .clicked()
        {
            start_export(app);
        }
        if let Some(e) = &app.export.err {
            ui.label(RichText::new(e.lines().next().unwrap_or("error")).small().color(Palette::DANGER));
        }
    });
}

/// Codec label with an availability dot for hardware encoders.
fn codec_label(app: &VeditApp, def: &export::VideoCodecDef) -> String {
    if def.hw {
        let dot = if app.hw_encoders.contains(def.id) { "●" } else { "○" };
        format!("{dot} {}", def.label)
    } else {
        def.label.to_string()
    }
}

fn display_output(settings: &ExportSettings) -> String {
    let mut p = settings.output.clone();
    if p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase())
        != Some(settings.container.to_string())
    {
        p.set_extension(settings.container);
    }
    p.display().to_string()
}

fn start_export(app: &mut VeditApp) {
    let Some(locator) = app.locator.clone() else {
        app.export.err = Some("ffmpeg not found — cannot export.".into());
        return;
    };
    let mut out = if app.export.out_path_text.trim().is_empty() {
        PathBuf::from("export")
    } else {
        PathBuf::from(app.export.out_path_text.trim())
    };
    if out.extension().map(|e| e.to_string_lossy().to_ascii_lowercase())
        != Some(app.export.settings.container.to_string())
    {
        out.set_extension(app.export.settings.container);
    }
    app.export.settings.output = out;

    match export::build_export_command(&locator, &app.project, &app.export.settings) {
        Ok(plan) => match ExportJob::start(plan) {
            Ok(job) => {
                app.export.done_msg = None;
                app.export.err = None;
                app.export.last_progress = 0.0;
                app.export.out_time = 0.0;
                app.export.job = Some(job);
                app.notify("Export started");
            }
            Err(e) => app.export.err = Some(e.to_string()),
        },
        Err(e) => app.export.err = Some(e.to_string()),
    }
}
