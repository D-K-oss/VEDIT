//! VEDIT application state + frame orchestration.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

use vedit_core::ffmpeg::FfmpegLocator;
use vedit_core::media::{MediaAsset, MediaKind};
use vedit_core::preview::{Evt, PlaybackEngine, PreviewSnapshot};
use vedit_core::Project;

use crate::audio::{AudioClipSpec, AudioPreviewer};
use crate::panels::export_dialog::ExportUi;
use crate::panels::timeline_ui::TimelineUiState;
use crate::panels;
use crate::settings::Settings;
use crate::theme;

#[cfg(debug_assertions)]
static UPDATE_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

// ---------------------------------------------------------------------------

/// Messages from background import/thumbnail/graphics threads.
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
    /// Audio peaks for waveform rendering (0..1 amplitudes).
    Peaks {
        id: u64,
        peaks: Option<Arc<Vec<f32>>>,
    },
    /// Filmstrip image (tiles laid out horizontally) for clip rendering.
    Strip {
        id: u64,
        strip: Option<Arc<image::RgbaImage>>,
        tiles: usize,
    },
    /// Encoders advertised by this ffmpeg build (for HW encode UI).
    Encoders(Vec<String>),
}

/// Filmstrip texture ready for timeline painting.
pub(crate) struct StripTex {
    pub tex: TextureHandle,
    pub tiles: usize,
}

/// Snapshot-based undo/redo. The whole project model is tiny (paths + clip
/// metadata), so cloning it per edit is cheap and always correct.
#[derive(Default)]
pub(crate) struct History {
    undo: Vec<Project>,
    redo: Vec<Project>,
    last_tag: Option<(String, Instant)>,
}

impl History {
    const CAP: usize = 100;

    pub(crate) fn push(&mut self, project: &Project, tag: &str) {
        // Coalesce rapid same-kind edits (drags, slider streams) so one
        // gesture = one undo step.
        if let Some((lt, at)) = &self.last_tag {
            if *lt == tag && at.elapsed() < Duration::from_millis(700) {
                return;
            }
        }
        self.last_tag = Some((tag.to_string(), Instant::now()));
        self.undo.push(project.clone());
        if self.undo.len() > Self::CAP {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub(crate) fn undo(&mut self, current: &Project) -> Option<Project> {
        let p = self.undo.pop()?;
        self.redo.push(current.clone());
        self.last_tag = None;
        Some(p)
    }

    pub(crate) fn redo(&mut self, current: &Project) -> Option<Project> {
        let p = self.redo.pop()?;
        self.undo.push(current.clone());
        self.last_tag = None;
        Some(p)
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.last_tag = None;
    }
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

    // v0.2.0: history, clip graphics, mixer, settings
    pub history: History,
    /// Audio peaks per asset (waveform rendering).
    pub waveforms: HashMap<u64, Arc<Vec<f32>>>,
    /// Filmstrip textures per asset.
    pub strips: HashMap<u64, StripTex>,
    /// Encoders available in the user's ffmpeg build.
    pub hw_encoders: HashSet<String>,
    pub mixer_open: bool,
    pub settings_open: bool,
    pub settings: Settings,
    pub autosave_last: Instant,

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
        let settings = Settings::load();

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
            preview_quality: PreviewQuality::ALL[settings.preview_quality_default.min(2)],
            audio: AudioPreviewer::new(),
            tl: TimelineUiState::default(),
            export: ExportUi::default(),
            new_dialog: NewProjectDialog {
                open: false,
                name: "Untitled Project".into(),
                res_index: 0,
                fps_index: 4,
            },
            history: History::default(),
            waveforms: HashMap::new(),
            strips: HashMap::new(),
            hw_encoders: HashSet::new(),
            mixer_open: false,
            settings_open: false,
            settings,
            autosave_last: Instant::now(),
            status: None,
            ffmpeg_version,
            autotest: matches!(
                std::env::var("VEDIT_AUTOTEST").as_deref(),
                Ok("1") | Ok("mixer") | Ok("settings") | Ok("export")
            ),
            autotest_started: false,
        };
        app.tl.snap = app.settings.snap_default;
        app.request_preview_frame();

        // Detect encoders once in the background (HW encode availability).
        if let Some(l) = app.locator.clone() {
            let tx = app.media_tx.clone();
            std::thread::spawn(move || {
                let set = vedit_core::export::available_encoders(&l);
                let _ = tx.send(MediaMsg::Encoders(set.into_iter().collect()));
            });
        }

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

    // -- undo / redo ---------------------------------------------------------

    /// Snapshot the project for undo. Call BEFORE a mutation. Tags coalesce
    /// rapid same-kind edits into a single undo step.
    pub(crate) fn push_undo(&mut self, tag: &str) {
        self.history.push(&self.project, tag);
    }

    pub(crate) fn do_undo(&mut self) {
        if let Some(p) = self.history.undo(&self.project) {
            let was_playing = self.playing;
            if was_playing {
                self.pause();
            }
            self.project = p;
            self.selected = None;
            self.dirty = true;
            self.request_preview_frame();
            self.notify("Undo");
        }
    }

    pub(crate) fn do_redo(&mut self) {
        if let Some(p) = self.history.redo(&self.project) {
            let was_playing = self.playing;
            if was_playing {
                self.pause();
            }
            self.project = p;
            self.selected = None;
            self.dirty = true;
            self.request_preview_frame();
            self.notify("Redo");
        }
    }

    /// Add a text/title clip at the playhead on the target video track.
    pub(crate) fn add_title_action(&mut self) {
        let Some(tid) = vedit_core::timeline::find_target_track(
            &self.project,
            vedit_core::project::TrackKind::Video,
            self.selected.map(|(t, _)| t),
        ) else {
            self.notify("No unlocked video track for a title");
            return;
        };
        self.push_undo("title");
        match vedit_core::timeline::add_title_clip(
            &mut self.project,
            tid,
            self.playhead,
            vedit_core::project::TextClipData::default(),
        ) {
            Ok(id) => {
                self.selected = Some((tid, id));
                self.mark_dirty();
                self.notify("Title added — edit it in the Inspector");
                self.request_preview_frame();
            }
            Err(e) => {
                self.history.undo(&self.project); // roll back the snapshot
                self.notify(format!("Could not add title: {e}"));
            }
        }
    }

    /// Audio clips audible at `time` (for preview playback).
    /// Video clips that have a linked audio twin are skipped — the twin on
    /// the audio track provides the sound (mixing both would double it).
    /// Volume folds in clip fades/keyframes × track fader × master.
    pub(crate) fn audio_specs(&self, time: f64) -> Vec<AudioClipSpec> {
        let any_solo = self.project.tracks.iter().any(|t| t.solo);
        let mut specs = Vec::new();
        for track in &self.project.tracks {
            let audible = !track.muted && (!any_solo || track.solo);
            if !audible {
                continue;
            }
            for clip in &track.clips {
                if clip.muted || clip.is_text() {
                    continue;
                }
                if track.kind == vedit_core::project::TrackKind::Video && clip.link.is_some() {
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
                let local_t = time - clip.start;
                let vol = clip.volume_at(local_t) * track.volume * self.project.master_volume;
                specs.push(AudioClipSpec {
                    clip_id: clip.id,
                    path: asset.path.clone(),
                    source_pos: clip.source_time_at(time),
                    volume: vol,
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
        let any_solo = self.project.tracks.iter().any(|t| t.solo);
        let has_audio = self.project.tracks.iter().any(|track| {
            !track.muted
                && (!any_solo || track.solo)
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
        self.request_asset_graphics();
    }

    /// Waveforms + filmstrips for every asset (background, cached on disk).
    pub(crate) fn request_asset_graphics(&mut self) {
        let Some(locator) = self.locator.clone() else { return };
        for asset in self.project.assets.clone() {
            let tx = self.media_tx.clone();
            let locator = locator.clone();
            std::thread::spawn(move || {
                if asset.has_audio() {
                    let dur = asset.duration().unwrap_or(0.0);
                    let peaks = vedit_core::waveform::extract_peaks(
                        locator.ffmpeg(),
                        &asset.path,
                        dur,
                        vedit_core::waveform::DEFAULT_BUCKETS,
                    )
                    .ok();
                    let _ = tx.send(MediaMsg::Peaks {
                        id: asset.id,
                        peaks: peaks.map(Arc::new),
                    });
                }
                match asset.kind {
                    MediaKind::Video => {
                        let dur = asset.duration().unwrap_or(1.0);
                        let strip = vedit_core::thumbnail::extract_filmstrip(&locator, &asset.path, dur).ok();
                        let _ = tx.send(MediaMsg::Strip {
                            id: asset.id,
                            strip: strip.map(Arc::new),
                            tiles: vedit_core::thumbnail::STRIP_TILES,
                        });
                    }
                    MediaKind::Image => {
                        if let Some(strip) = vedit_core::thumbnail::image_filmstrip(&asset.path) {
                            let _ = tx.send(MediaMsg::Strip {
                                id: asset.id,
                                strip: Some(Arc::new(strip)),
                                tiles: vedit_core::thumbnail::STRIP_TILES,
                            });
                        }
                    }
                    MediaKind::Audio => {}
                }
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
                MediaMsg::Peaks { id, peaks } => {
                    if let Some(p) = peaks {
                        self.waveforms.insert(id, p);
                    }
                }
                MediaMsg::Strip { id, strip, tiles } => {
                    if let Some(img) = strip {
                        let tex = ctx.load_texture(
                            format!("strip-{id}"),
                            ColorImage::from_rgba_unmultiplied(
                                [img.width() as usize, img.height() as usize],
                                img.as_raw(),
                            ),
                            TextureOptions::LINEAR,
                        );
                        self.strips.insert(id, StripTex { tex, tiles });
                    }
                }
                MediaMsg::Encoders(list) => {
                    self.hw_encoders = list.into_iter().collect();
                }
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
            // Kick off waveform/filmstrip generation for the new asset.
            self.request_asset_graphics();
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
        // Bound the drain per UI frame: the engine keeps producing while we
        // convert frames, so an unbounded `while let` may never see an empty
        // queue on slow machines — update() would never return and the UI
        // would freeze on the last painted frame. Leftover events are picked
        // up on the next repaint (~16 ms later); only the LAST video frame
        // of a batch is uploaded to the GPU (intermediate ones are dropped).
        const MAX_DRAIN: usize = 8;
        #[cfg(debug_assertions)]
        let mut frames = 0usize;
        let mut last_frame: Option<(f64, std::sync::Arc<Vec<u8>>)> = None;
        let mut last_audio_time: Option<f64> = None;
        for _ in 0..MAX_DRAIN {
            let Some(evt) = self.engine.try_recv() else { break };
            #[cfg(debug_assertions)]
            {
                frames += 1;
                if let Evt::Ended { time } = &evt {
                    eprintln!("[vedit] evt Ended t={time:.3}");
                }
            }
            match evt {
                Evt::Frame { time, rgba } => {
                    self.playhead = time;
                    self.seek_inflight = false;
                    last_frame = Some((time, rgba));
                    last_audio_time = Some(time);
                }
                Evt::Tick { time } => {
                    self.playhead = time;
                    last_audio_time = Some(time);
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
        if let Some(t) = last_audio_time {
            if self.playing {
                let specs = self.audio_specs(t);
                self.audio.update(&specs);
            }
        }
        if let Some((_, rgba)) = last_frame {
            self.update_preview_texture(ctx, &rgba);
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
                self.waveforms.clear();
                self.strips.clear();
                self.history.clear();
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
        self.waveforms.clear();
        self.strips.clear();
        self.history.clear();
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
            // Undo / redo
            if i.modifiers.ctrl && i.key_pressed(egui::Key::Z) {
                if i.modifiers.shift {
                    self.do_redo();
                } else {
                    self.do_undo();
                }
            }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::Y) {
                self.do_redo();
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
                    self.push_undo("split");
                    // Link-aware: a linked A/V pair is cut together.
                    if !vedit_core::timeline::split_linked(&mut self.project, track_id, clip_id, t).is_empty() {
                        split_any = true;
                    } else {
                        self.history.undo(&self.project); // nothing was cut
                    }
                }
            }
        }
        if !split_any {
            let any_intersect = self
                .project
                .tracks
                .iter()
                .any(|tr| !tr.locked && tr.clip_at(t).is_some());
            if any_intersect {
                self.push_undo("split");
            }
            split_any = !vedit_core::timeline::split_all_at(&mut self.project, t).is_empty();
            if !split_any && any_intersect {
                self.history.undo(&self.project);
            }
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
            let locked = self.project.track(track_id).map(|t| t.locked).unwrap_or(false);
            if locked {
                self.notify("Track is locked — unlock it to delete clips");
                return;
            }
            self.push_undo("delete");
            // Link-aware: the linked A/V partner is removed too.
            let n = vedit_core::timeline::remove_clip_with_link(&mut self.project, track_id, clip_id);
            if n > 0 {
                self.mark_dirty();
                if n > 1 {
                    self.notify("Linked A/V clips removed");
                } else {
                    self.notify("Clip removed");
                }
                self.request_preview_frame();
            } else {
                self.history.undo(&self.project);
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
        panels::mixer(self, ctx);
        panels::inspector(self, ctx);
        panels::preview(self, ctx);

        // Headless smoke test: start playback once the UI is up.
        if self.autotest && !self.autotest_started {
            self.autotest_started = true;
            self.play();
            // VEDIT_AUTOTEST may also pre-open panels for headless UI checks:
            //   "1" (or empty) = play only, "mixer" = play + mixer + select,
            //   "settings" = play + settings dialog, "export" = play + export.
            match std::env::var("VEDIT_AUTOTEST").as_deref() {
                Ok("mixer") => {
                    self.mixer_open = true;
                    // Select the first clip so the inspector shows clip UI.
                    for t in &self.project.tracks {
                        if let Some(c) = t.clips.first() {
                            self.selected = Some((t.id, c.id));
                            break;
                        }
                    }
                }
                Ok("settings") => self.settings_open = true,
                Ok("export") => self.export.open = true,
                _ => {}
            }
        }

        // Dialogs + keys
        self.handle_keys(ctx);
        panels::export_dialog(self, ctx);
        panels::new_project_dialog(self, ctx);
        panels::settings_dialog(self, ctx);

        // Autosave: background-save every N minutes while dirty.
        if self.settings.autosave_enabled
            && self.dirty
            && self.autosave_last.elapsed()
                >= Duration::from_secs(self.settings.autosave_minutes.max(1) as u64 * 60)
        {
            self.autosave_last = Instant::now();
            let target = self
                .project_path
                .clone()
                .map(|p| {
                    let mut name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    name.push_str(".autosave.vedit.json");
                    p.with_file_name(name)
                })
                .unwrap_or_else(|| std::env::temp_dir().join("vedit-autosave.vedit.json"));
            if self.project.save(&target).is_ok() {
                self.notify(format!("Autosaved → {}", target.display()));
            }
        }

        // Keep painting while something moves; low-rate idle repaint otherwise.
        if self.playing || self.export.job_running() {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}
