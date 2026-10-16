//! VEDIT application state + frame orchestration.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

use vedit_core::ffmpeg::FfmpegLocator;
use vedit_core::media::MediaAsset;
use vedit_core::preview::{Evt, PlaybackEngine, PreviewSnapshot};
use vedit_core::Project;

use crate::audio::{AudioClipSpec, AudioPreviewer};
use crate::panels::export_dialog::ExportUi;
use crate::panels::timeline_ui::TimelineUiState;
use crate::panels;
use crate::theme;

#[cfg(debug_assertions)]
static UPDATE_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

// ---------------------------------------------------------------------------

/// Messages from background import/thumbnail threads.
pub(crate) enum MediaMsg {
    Imported {
        asset: Option<MediaAsset>,
        thumb: Option<image::RgbaImage>,
        error: Option<String>,
    },
    Thumb {
        id: u64,
        thumb: Option<image::RgbaImage>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PreviewQuality {
    Full,
    Half,
    Quarter,
}

impl PreviewQuality {
    pub fn scale(&self) -> f32 {
        match self {
            PreviewQuality::Full => 1.0,
            PreviewQuality::Half => 0.5,
            PreviewQuality::Quarter => 0.25,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            PreviewQuality::Full => "Full",
            PreviewQuality::Half => "Half",
            PreviewQuality::Quarter => "Quarter",
        }
    }
    pub const ALL: [PreviewQuality; 3] = [PreviewQuality::Full, PreviewQuality::Half, PreviewQuality::Quarter];
}

pub(crate) struct NewProjectDialog {
    pub open: bool,
    pub name: String,
    pub res_index: usize,
    pub fps_index: usize,
}

pub const RES_PRESETS: &[( &str, u32, u32)] = &[
    ("1920 × 1080 (HD 16:9)", 1920, 1080),
    ("1280 × 720 (HD)", 1280, 720),
    ("3840 × 2160 (4K UHD)", 3840, 2160),
    ("2560 × 1440 (QHD)", 2560, 1440),
    ("1080 × 1920 (Vertical 9:16)", 1080, 1920),
    ("1080 × 1080 (Square)", 1080, 1080),
];

pub const FPS_PRESETS: &[f64] = &[23.976, 24.0, 25.0, 29.97, 30.0, 50.0, 59.94, 60.0];

// ---------------------------------------------------------------------------

pub(crate) struct VeditApp {
    pub project: Project,
    pub project_path: Option<PathBuf>,
    pub dirty: bool,
    pub locator: Option<FfmpegLocator>,

    // Media library
    pub thumbs: HashMap<u64, TextureHandle>,
    pub media_tx: Sender<MediaMsg>,
    pub media_rx: Receiver<MediaMsg>,
    pub pending_imports: usize,

    // Selection & transport
    pub selected: Option<(u64, u64)>, // (track_id, clip_id)
    pub playhead: f64,
    pub playing: bool,
    pub seek_inflight: bool,

    // Preview
    pub engine: PlaybackEngine,
    pub preview_tex: Option<TextureHandle>,
    pub preview_size: (u32, u32),
    pub preview_quality: PreviewQuality,

    // Audio preview
    pub audio: AudioPreviewer,

    // Timeline UI
    pub tl: TimelineUiState,

    // Dialogs
    pub export: ExportUi,
    pub new_dialog: NewProjectDialog,

    // Transient status message
    pub status: Option<(String, Instant)>,

    // Cached ffmpeg version string for the status bar
    pub ffmpeg_version: Option<String>,

    // Headless smoke-testing hook (VEDIT_AUTOTEST=1 → auto-play at startup)
    pub autotest: bool,
    pub autotest_started: bool,
}

impl VeditApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        locator: Option<FfmpegLocator>,
        project_arg: Option<PathBuf>,
    ) -> Self {
        theme::apply(&cc.egui_ctx);

        let (tx, rx) = std::sync::mpsc::channel::<MediaMsg>();
        let engine_path = locator
            .as_ref()
            .map(|l| l.ffmpeg().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("ffmpeg"));
        let ffmpeg_version = locator.as_ref().map(|l| l.version());

        let mut app = VeditApp {
            project: Project::default(),
            project_path: None,
            dirty: false,
            locator,
            thumbs: HashMap::new(),
            media_tx: tx,
            media_rx: rx,
            pending_imports: 0,
            selected: None,
            playhead: 0.0,
            playing: false,
            seek_inflight: false,
            engine: PlaybackEngine::spawn(engine_path),
            preview_tex: None,
            preview_size: (960, 540),
            preview_quality: PreviewQuality::Half,
            audio: AudioPreviewer::new(),
            tl: TimelineUiState::default(),
            export: ExportUi::default(),
            new_dialog: NewProjectDialog {
                open: false,
                name: "Untitled Project".into(),
                res_index: 0,
                fps_index: 4,
            },
            status: None,
            ffmpeg_version,
            autotest: std::env::var("VEDIT_AUTOTEST").as_deref() == Ok("1"),
            autotest_started: false,
        };
        app.request_preview_frame();

        // Open a project passed on the command line (file association support).
        if let Some(path) = project_arg {
            if path.is_file() {
                app.load_project_from(path);
            }
        }
        app
    }

    // -- small helpers ------------------------------------------------------

    pub(crate) fn notify(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Audio clips audible at `time` (for preview playback).
    pub(crate) fn audio_specs(&self, time: f64) -> Vec<AudioClipSpec> {
        let mut specs = Vec::new();
        for track in &self.project.tracks {
            for clip in &track.clips {
                if track.muted || clip.muted {
                    continue;
                }
                if time < clip.start || time >= clip.end() {
                    continue;
                }
                let Some(asset) = self.project.asset(clip.asset_id) else {
                    continue;
                };
                if !asset.has_audio() {
                    continue;
                }
                specs.push(AudioClipSpec {
                    clip_id: clip.id,
                    path: asset.path.clone(),
                    source_pos: clip.source_time_at(time),
                    volume: clip.volume,
                    speed: clip.speed,
                });
            }
        }
        specs
    }

    /// Ask the engine to render the paused frame at the playhead.
    pub(crate) fn request_preview_frame(&mut self) {
        if self.seek_inflight {
            return;
        }
        let snap: Option<PreviewSnapshot> =
            vedit_core::preview::snapshot_at(&self.project, self.playhead, self.preview_quality.scale());
        if let Some(s) = &snap {
            self.preview_size = (s.width, s.height);
        } else {
            // Nothing under the playhead — drop the stale frame so the
            // placeholder shows instead.
            self.preview_tex = None;
        }
        if let Some(s) = snap {
            self.seek_inflight = true;
            self.engine.send(vedit_core::preview::Cmd::Seek {
                time: self.playhead,
                snapshot: Some(s),
            });
        }
    }

    fn update_preview_texture(&mut self, ctx: &egui::Context, rgba: &[u8]) {
        let (w, h) = self.preview_size;
        let expected = w as usize * h as usize * 4;
        if rgba.len() != expected {
            return;
        }
        let img = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba);
        let needs_new = match &self.preview_tex {
            Some(tex) => tex.size() != [w as usize, h as usize],
            None => true,
        };
        if needs_new {
            self.preview_tex = Some(ctx.load_texture("preview", img, TextureOptions::LINEAR));
        } else if let Some(tex) = self.preview_tex.as_mut() {
            tex.set(img, TextureOptions::LINEAR);
        }
    }

    // -- transport -----------------------------------------------------------

    pub(crate) fn play(&mut self) {
        if self.playing {
            return;
        }
        let t = self.playhead;
        let snap = vedit_core::preview::snapshot_at(&self.project, t, self.preview_quality.scale());
        let has_video = snap.is_some();
        let audio_specs = self.audio_specs(t);
        #[cfg(debug_assertions)]
        let audio_specs_len = audio_specs.len();
        // Any audible clip anywhere on the timeline?
        let has_audio = self.project.tracks.iter().any(|track| {
            !track.muted
                && track
                    .clips
                    .iter()
                    .any(|clip| {
                        !clip.muted
                            && self
                                .project
                                .asset(clip.asset_id)
                                .map(|a| a.has_audio())
                                .unwrap_or(false)
                    })
        });
        if !has_video && !has_audio && self.project.duration() <= t {
            self.notify("Nothing under the playhead — move it onto a clip first.");
            return;
        }
        if let Some(s) = &snap {
            self.preview_size = (s.width, s.height);
        }
        let end_time = self.project.duration().max(t + 0.1);
        self.audio.start(audio_specs);
        #[cfg(debug_assertions)]
        eprintln!(
            "[vedit] play() t={t:.3} video_layers={} audio_clips={} end={end_time:.3}",
            if has_video { 1 } else { 0 },
            audio_specs_len,
        );
        self.engine.send(vedit_core::preview::Cmd::Play {
            time: t,
            video: snap,
            end_time,
        });
        self.playing = true;
    }

    pub(crate) fn pause(&mut self) {
        if !self.playing {
            return;
        }
        self.playing = false;
        self.audio.stop();
        self.engine.send(vedit_core::preview::Cmd::Pause);
        self.request_preview_frame();
    }

    pub(crate) fn toggle_play(&mut self) {
        #[cfg(debug_assertions)]
        eprintln!("[vedit] toggle_play (playing={})", self.playing);
        if self.playing {
            self.pause();
        } else {
            self.play();
        }
    }

    pub(crate) fn step_frames(&mut self, frames: f64) {
        if self.playing {
            self.pause();
        }
        let dt = frames / self.project.fps.max(1.0);
        self.playhead = (self.playhead + dt).clamp(0.0, self.project.duration());
        self.request_preview_frame();
    }

    pub(crate) fn seek_to(&mut self, t: f64) {
        let t = t.clamp(0.0, self.project.duration());
        if self.playing {
            self.pause();
        }
        self.playhead = t;
        self.request_preview_frame();
    }

    // -- import ----------------------------------------------------------------

    pub(crate) fn import_paths(&mut self, paths: Vec<PathBuf>) {
        let Some(locator) = self.locator.clone() else {
            self.notify("ffmpeg not found — cannot import media. See the banner in the preview panel.");
            return;
        };
        for path in paths {
            if !path.is_file() {
                continue;
            }
            let id = self.project.alloc_id();
            let tx = self.media_tx.clone();
            let locator = locator.clone();
            self.pending_imports += 1;
            std::thread::spawn(move || {
                let result = vedit_core::probe::probe(&locator, &path);
                match result {
                    Ok(info) => match vedit_core::media::make_asset(id, path, info) {
                        Ok(asset) => {
                            let thumb = vedit_core::thumbnail::generate_thumbnail(&locator, &asset);
                            let _ = tx.send(MediaMsg::Imported {
                                asset: Some(asset),
                                thumb,
                                error: None,
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(MediaMsg::Imported {
                                asset: None,
                                thumb: None,
                                error: Some(e.to_string()),
                            });
                        }
                    },
                    Err(e) => {
                        let _ = tx.send(MediaMsg::Imported {
                            asset: None,
                            thumb: None,
                            error: Some(e.to_string()),
                        });
                    }
                }
            });
        }
    }

    /// Regenerate library thumbnails (used after loading a project).
    fn request_thumbnails(&mut self) {
        let Some(locator) = self.locator.clone() else { return };
        for asset in self.project.assets.clone() {
            let tx = self.media_tx.clone();
            let locator = locator.clone();
            std::thread::spawn(move || {
                let thumb = vedit_core::thumbnail::generate_thumbnail(&locator, &asset);
                let _ = tx.send(MediaMsg::Thumb { id: asset.id, thumb });
            });
        }
    }

    // -- per-frame polling -------------------------------------------------------

    fn poll_media(&mut self, ctx: &egui::Context) {
        let mut new_assets: Vec<(MediaAsset, Option<image::RgbaImage>)> = Vec::new();
        let mut thumbs: Vec<(u64, Option<image::RgbaImage>)> = Vec::new();
        while let Ok(msg) = self.media_rx.try_recv() {
            match msg {
                MediaMsg::Imported { asset, thumb, error } => {
                    self.pending_imports = self.pending_imports.saturating_sub(1);
                    if let Some(err) = error {
                        self.notify(format!("Import failed: {err}"));
                    } else if let Some(a) = asset {
                        new_assets.push((a, thumb));
                    }
                }
                MediaMsg::Thumb { id, thumb } => thumbs.push((id, thumb)),
            }
        }

        for (asset, thumb) in new_assets {
            let id = asset.id;
            let label = asset.name.clone();
            self.project.assets.push(asset);
            if let Some(img) = thumb {
                let tex = ctx.load_texture(
                    format!("thumb-{id}"),
                    ColorImage::from_rgba_unmultiplied(
                        [img.width() as usize, img.height() as usize],
                        img.as_raw(),
                    ),
                    TextureOptions::LINEAR,
                );
                self.thumbs.insert(id, tex);
            }
            self.dirty = true;
            self.notify(format!("Imported {label}"));
        }
        for (id, thumb) in thumbs {
            if let (Some(img), false) = (thumb, self.thumbs.contains_key(&id)) {
                let tex = ctx.load_texture(
                    format!("thumb-{id}"),
                    ColorImage::from_rgba_unmultiplied(
                        [img.width() as usize, img.height() as usize],
                        img.as_raw(),
                    ),
                    TextureOptions::LINEAR,
                );
                self.thumbs.insert(id, tex);
            }
        }
    }

    fn poll_engine(&mut self, ctx: &egui::Context) {
        #[cfg(debug_assertions)]
        let mut frames = 0usize;
        while let Some(evt) = self.engine.try_recv() {
            #[cfg(debug_assertions)]
            {
                frames += 1;
                if let Evt::Frame { time, .. } = &evt {
                    if *time < 0.2 || (frames % 30) == 0 {
                        eprintln!("[vedit] evt Frame t={time:.3}");
                    }
                }
                if let Evt::Ended { time } = &evt {
                    eprintln!("[vedit] evt Ended t={time:.3}");
                }
            }
            match evt {
                Evt::Frame { time, rgba } => {
                    self.playhead = time;
                    self.seek_inflight = false;
                    self.update_preview_texture(ctx, &rgba);
                    if self.playing {
                        let specs = self.audio_specs(time);
                        self.audio.update(&specs);
                    }
                }
                Evt::Tick { time } => {
                    self.playhead = time;
                    if self.playing {
                        let specs = self.audio_specs(time);
                        self.audio.update(&specs);
                    }
                }
                Evt::NeedSnapshot { time } => {
                    // Playback crossed a clip boundary: rebuild decoders for
                    // the next segment and keep rolling seamlessly.
                    self.playhead = time;
                    let snap = vedit_core::preview::snapshot_at(
                        &self.project,
                        time,
                        self.preview_quality.scale(),
                    );
                    if let Some(s) = &snap {
                        self.preview_size = (s.width, s.height);
                    }
                    let end_time = self.project.duration().max(time + 0.1);
                    self.engine.send(vedit_core::preview::Cmd::Play {
                        time,
                        video: snap,
                        end_time,
                    });
                }
                Evt::Ended { time } => {
                    self.playing = false;
                    self.audio.stop();
                    self.playhead = (time - 1.0 / self.project.fps.max(1.0)).max(0.0);
                    self.request_preview_frame();
                }
                Evt::Error(msg) => {
                    self.seek_inflight = false;
                    self.notify(msg);
                }
            }
        }
        #[cfg(debug_assertions)]
        if frames > 0 {
            log::debug!("polled {frames} engine events");
        }
    }

    fn poll_export(&mut self) {
        let mut done: Option<Result<PathBuf, String>> = None;
        if let Some(job) = &mut self.export.job {
            while let Some(evt) = job.poll() {
                match evt {
                    vedit_core::export::ExportEvt::Progress { pct, out_time } => {
                        self.export.last_progress = pct;
                        self.export.out_time = out_time;
                    }
                    vedit_core::export::ExportEvt::Done { output } => {
                        done = Some(Ok(output));
                    }
                    vedit_core::export::ExportEvt::Failed { message } => {
                        done = Some(Err(message));
                    }
                }
            }
        }
        if let Some(res) = done {
            match res {
                Ok(p) => {
                    self.export.done_msg = Some(p.display().to_string());
                    self.notify("Export finished");
                }
                Err(e) => {
                    self.export.err = Some(e);
                    self.notify("Export failed");
                }
            }
            self.export.last_progress = 100.0;
        }
    }

    // -- files --------------------------------------------------------------------

    pub(crate) fn save_project(&mut self, save_as: bool) {
        let path = match (&self.project_path, save_as) {
            (Some(p), false) => p.clone(),
            _ => {
                let Some(p) = rfd::FileDialog::new()
                    .set_title("Save VEDIT Project")
                    .add_filter("VEDIT project", &["vedit.json"])
                    .set_file_name(format!("{}.vedit.json", self.project.name))
                    .save_file()
                else {
                    return;
                };
                self.project_path = Some(p);
                self.project_path.clone().unwrap()
            }
        };
        match self.project.save(&path) {
            Ok(()) => {
                self.project_path = Some(path);
                self.dirty = false;
                self.notify("Project saved");
            }
            Err(e) => self.notify(format!("Save failed: {e}")),
        }
    }

    pub(crate) fn open_project_dialog(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open VEDIT Project")
            .add_filter("VEDIT project", &["vedit.json"])
            .pick_file()
        else {
            return;
        };
        self.load_project_from(path);
    }

    pub(crate) fn load_project_from(&mut self, path: PathBuf) {
        match Project::load(&path) {
            Ok((project, p)) => {
                self.pause();
                self.project = project;
                self.project_path = Some(p);
                self.dirty = false;
                self.playhead = 0.0;
                self.selected = None;
                self.thumbs.clear();
                self.seek_inflight = false;
                self.request_thumbnails();
                self.request_preview_frame();
                self.notify(format!("Opened {}", self.project.name));
            }
            Err(e) => self.notify(format!("Open failed: {e}")),
        }
    }

    pub(crate) fn create_new_project(&mut self, name: &str, w: u32, h: u32, fps: f64) {
        self.pause();
        self.project = Project::new(name, w, h, fps);
        self.project_path = None;
        self.dirty = false;
        self.playhead = 0.0;
        self.selected = None;
        self.thumbs.clear();
        self.seek_inflight = false;
        self.request_preview_frame();
        self.notify(format!("New project: {name}"));
    }

    // -- keyboard ---------------------------------------------------------------

    fn handle_keys(&mut self, ctx: &egui::Context) {
        if self.export.open || self.new_dialog.open {
            return;
        }
        let typing = ctx.wants_keyboard_input();
        ctx.input(|i| {
            if typing && i.key_pressed(egui::Key::Space) {
                return;
            }
            // Space — play/pause
            if i.key_pressed(egui::Key::Space) && !typing {
                self.toggle_play();
            }
            // Split
            if i.key_pressed(egui::Key::S) && !i.modifiers.ctrl && !i.modifiers.shift && !i.modifiers.alt && !typing {
                self.split_action();
            }
            // Delete clip
            if (i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) && !typing {
                self.delete_selected();
            }
            // Frame stepping
            if i.key_pressed(egui::Key::ArrowLeft) {
                let frames = if i.modifiers.shift { -10.0 } else { -1.0 };
                self.step_frames(frames);
            }
            if i.key_pressed(egui::Key::ArrowRight) {
                let frames = if i.modifiers.shift { 10.0 } else { 1.0 };
                self.step_frames(frames);
            }
            if i.key_pressed(egui::Key::Home) {
                self.seek_to(0.0);
            }
            if i.key_pressed(egui::Key::End) {
                self.seek_to(self.project.duration());
            }
            // Ctrl shortcuts
            if i.modifiers.ctrl && i.key_pressed(egui::Key::S) {
                self.save_project(false);
            }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::E) {
                self.export.open = true;
            }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::O) {
                self.open_project_dialog();
            }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::I) {
                self.import_media_dialog();
            }
        });
    }

    pub(crate) fn split_action(&mut self) {
        if self.playing {
            self.pause();
        }
        let t = self.playhead;
        let mut split_any = false;
        if let Some((track_id, clip_id)) = self.selected {
            if let Some(clip) = self.project.find_clip(track_id, clip_id) {
                if t > clip.start && t < clip.end() {
                    let tid = track_id;
                    if vedit_core::timeline::split_at(&mut self.project, tid, t).is_some() {
                        split_any = true;
                    }
                }
            }
        }
        if !split_any {
            split_any = !vedit_core::timeline::split_all_at(&mut self.project, t).is_empty();
        }
        if split_any {
            self.mark_dirty();
            self.notify("Split at playhead");
            self.request_preview_frame();
        } else {
            self.notify("Nothing to split at the playhead");
        }
    }

    pub(crate) fn delete_selected(&mut self) {
        if let Some((track_id, clip_id)) = self.selected.take() {
            if vedit_core::timeline::remove_clip(&mut self.project, track_id, clip_id) {
                self.mark_dirty();
                self.notify("Clip removed");
                self.request_preview_frame();
            }
        }
    }

    pub(crate) fn import_media_dialog(&mut self) {
        let paths = rfd::FileDialog::new()
            .set_title("Import media")
            .add_filter("Media files", vedit_core::media::IMPORT_DIALOG_EXTS)
            .pick_files();
        if let Some(paths) = paths {
            self.import_paths(paths);
        }
    }
}

// ---------------------------------------------------------------------------

impl eframe::App for VeditApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(debug_assertions)]
        {
            UPDATE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let n = UPDATE_COUNT.load(std::sync::atomic::Ordering::Relaxed);
            if n % 120 == 1 {
                eprintln!("[vedit] update #{n} playing={} playhead={:.3}", self.playing, self.playhead);
            }
        }
        // Background results
        self.poll_media(ctx);
        self.poll_engine(ctx);
        self.poll_export();

        // OS drag & drop import
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            self.import_paths(dropped);
        }

        // Panels — side/top/bottom panels first, CentralPanel LAST (egui
        // convention: otherwise bottom panels overlap central content).
        panels::top_bar(self, ctx);
        panels::status_bar(self, ctx);
        panels::timeline(self, ctx);
        panels::media_library(self, ctx);
        panels::inspector(self, ctx);
        panels::preview(self, ctx);

        // Headless smoke test: start playback once the UI is up.
        if self.autotest && !self.autotest_started {
            self.autotest_started = true;
            self.play();
        }

        // Dialogs + keys
        self.handle_keys(ctx);
        panels::export_dialog(self, ctx);
        panels::new_project_dialog(self, ctx);

        // Keep painting while something moves; low-rate idle repaint otherwise.
        if self.playing || self.export.job_running() {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}
