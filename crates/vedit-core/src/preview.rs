//! Frame-accurate preview: composites the active video layers into RGBA
//! frames and streams them to the GUI.
//!
//! Playback uses one streaming `ffmpeg -f rawvideo` decoder per active layer;
//! scrubbing decodes a single frame on demand. Compositing (straight-alpha
//! over, per-layer opacity curves, text overlays, color grades) happens on
//! the engine thread so the UI never blocks.

use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Stdio};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::colorgrade::GradeTransform;
use crate::ffmpeg::{base_command, kill_child};
use crate::project::{Curve, Project, TextClipData};

// ---------------------------------------------------------------------------
// Snapshot (what the GUI asks the engine to render)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PreviewClip {
    pub clip_id: u64,
    pub asset_path: PathBuf,
    pub is_image: bool,
    /// Text/title layer: no decoder, `text` is rasterized once per session.
    pub is_text: bool,
    pub text: Option<TextClipData>,
    pub in_point: f64,
    pub source_duration: f64,
    /// Effective timeline start (includes cross-dissolve pull-in).
    pub start: f64,
    /// Effective duration.
    pub duration: f64,
    pub speed: f64,
    /// Opacity over the layer's local timeline (keyframes + fades + dissolve).
    pub opacity_curve: Curve,
    /// Per-layer color grade (None = neutral).
    pub grade: Option<GradeTransform>,
}

#[derive(Debug, Clone)]
pub struct PreviewSnapshot {
    /// Bottom → top layer order.
    pub layers: Vec<PreviewClip>,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

/// Build a snapshot of every video clip active at `time` (bottom → top).
/// `quality_scale` downscales preview rendering (1.0 / 0.5 / 0.25).
///
/// The result is additionally clamped to `MAX_PREVIEW_EDGE` on the long edge:
/// decoding and texture-uploading e.g. full 4K frames per UI frame saturates
/// memory bandwidth (33 MB/frame) and can exceed GPU texture limits — that
/// is what made 4K timelines freeze or crash. Exports are unaffected: they
/// run through ffmpeg at full project resolution.
pub const MAX_PREVIEW_EDGE: u32 = 1920;

pub fn snapshot_at(project: &Project, time: f64, quality_scale: f32) -> Option<PreviewSnapshot> {
    let mut layers = Vec::new();
    // Display order is top → bottom, so composite from the end.
    for track in project.tracks.iter().rev().filter(|t| t.kind == crate::project::TrackKind::Video) {
        for (idx, clip) in track.clips.iter().enumerate() {
            // Cross-dissolve pull-in: this layer is also active while the
            // PREVIOUS clip's dissolve window is running.
            let pull = if idx > 0 {
                crate::timeline::effective_overlap(project, track.id, &track.clips[idx - 1])
            } else {
                0.0
            };
            if time < clip.start - pull - 1e-6 || time >= clip.end() {
                continue;
            }
            let own_extra = crate::timeline::effective_overlap(project, track.id, clip);
            let grade = clip
                .grade
                .as_ref()
                .map(crate::colorgrade::GradeTransform::build_lenient);

            if clip.is_text() {
                let text = clip.text.clone().unwrap_or_default();
                layers.push(PreviewClip {
                    clip_id: clip.id,
                    asset_path: PathBuf::new(),
                    is_image: true,
                    is_text: true,
                    text: Some(text),
                    in_point: 0.0,
                    source_duration: 0.0,
                    start: clip.start,
                    duration: clip.duration,
                    speed: clip.speed,
                    opacity_curve: clip.opacity_curve(own_extra),
                    grade,
                });
                continue;
            }

            let Some(asset) = project.asset(clip.asset_id) else { continue };
            if asset.kind != crate::media::MediaKind::Video && asset.kind != crate::media::MediaKind::Image {
                continue;
            }
            let (start, in_point, source_duration, duration) = if pull > 0.0 {
                (
                    clip.start - pull,
                    (clip.in_point - pull * clip.speed).max(0.0),
                    clip.source_duration() + pull * clip.speed,
                    clip.duration + pull,
                )
            } else {
                (clip.start, clip.in_point, clip.source_duration(), clip.duration)
            };
            let curve: Curve = if pull > 0.0 {
                let mut c: Curve = vec![(0.0, 0.0)];
                for (t, v) in clip.opacity_curve(own_extra) {
                    c.push((t + pull, v));
                }
                c
            } else {
                clip.opacity_curve(own_extra)
            };
            layers.push(PreviewClip {
                clip_id: clip.id,
                asset_path: asset.path.clone(),
                is_image: asset.kind == crate::media::MediaKind::Image,
                is_text: false,
                text: None,
                in_point,
                source_duration,
                start,
                duration,
                speed: clip.speed,
                opacity_curve: curve,
                grade,
            });
        }
    }
    if layers.is_empty() {
        return None;
    }
    // Fold curves to f32-friendly precision (values are 0..1, times seconds).
    let long_edge = project.width.max(project.height).max(1) as f32;
    let cap_scale = (MAX_PREVIEW_EDGE as f32 / long_edge).min(1.0);
    let scale = quality_scale.min(cap_scale);
    let even = |v: u32| (v as f32 * scale).round().max(16.0) as u32 / 2 * 2;
    let (w, h) = (even(project.width), even(project.height));

    // Scale text sizes from project pixels to preview pixels.
    let text_scale = w as f64 / project.width.max(1) as f64;
    for layer in &mut layers {
        if let Some(text) = &mut layer.text {
            text.size = (text.size * text_scale).max(6.0);
        }
    }

    Some(PreviewSnapshot {
        layers,
        width: w,
        height: h,
        fps: project.fps,
    })
}

// ---------------------------------------------------------------------------
// Engine protocol
// ---------------------------------------------------------------------------

pub enum Cmd {
    /// Start playback. `video` may be `None` for audio-only playback.
    Play { time: f64, video: Option<PreviewSnapshot>, end_time: f64 },
    Pause,
    /// Render a single frame while paused.
    Seek { time: f64, snapshot: Option<PreviewSnapshot> },
    Shutdown,
}

pub enum Evt {
    Frame { time: f64, rgba: Arc<Vec<u8>> },
    /// Progress tick without video (audio-only playback).
    Tick { time: f64 },
    /// All layer decoders reached EOF but the timeline continues —
    /// the GUI should build a fresh snapshot and send `Play` again.
    NeedSnapshot { time: f64 },
    Ended { time: f64 },
    Error(String),
}

// ---------------------------------------------------------------------------
// Decoder helpers
// ---------------------------------------------------------------------------

fn scale_chain(w: u32, h: u32, fps: f64) -> String {
    format!(
        "fps={fps:.4},scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,format=rgba",
        w = w,
        h = h
    )
}

fn spawn_stream_decoder(
    ffmpeg: &Path,
    clip: &PreviewClip,
    w: u32,
    h: u32,
    fps: f64,
    seek_local: f64,
) -> std::io::Result<(Child, BufReader<ChildStdout>)> {
    let mut cmd = base_command(ffmpeg);
    cmd.arg("-hide_banner").arg("-loglevel").arg("error");
    if clip.is_image {
        let remain = (clip.duration - seek_local).max(0.05);
        cmd.arg("-loop").arg("1").arg("-t").arg(format!("{:.3}", remain));
    } else {
        let seek_src = (seek_local * clip.speed).max(0.0);
        cmd.arg("-ss")
            .arg(format!("{:.3}", clip.in_point + seek_src));
        cmd.arg("-t")
            .arg(format!("{:.3}", (clip.source_duration - seek_src).max(0.05)));
    }
    cmd.arg("-i").arg(&clip.asset_path);
    cmd.arg("-vf").arg(scale_chain(w, h, fps));
    cmd.arg("-f").arg("rawvideo").arg("-pix_fmt").arg("rgba").arg("pipe:1");
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take().expect("stdout piped");
    Ok((child, BufReader::with_capacity(w as usize * h as usize * 4, stdout)))
}

fn spawn_oneshot_decoder(
    ffmpeg: &Path,
    clip: &PreviewClip,
    w: u32,
    h: u32,
    fps: f64,
    src_pos: f64,
) -> std::io::Result<Child> {
    let mut cmd = base_command(ffmpeg);
    cmd.arg("-hide_banner").arg("-loglevel").arg("error");
    if clip.is_image {
        cmd.arg("-loop").arg("1");
    } else {
        cmd.arg("-ss").arg(format!("{:.3}", src_pos.max(0.0)));
    }
    cmd.arg("-i").arg(&clip.asset_path);
    cmd.arg("-frames:v").arg("1");
    cmd.arg("-vf").arg(scale_chain(w, h, fps));
    cmd.arg("-f").arg("rawvideo").arg("-pix_fmt").arg("rgba").arg("pipe:1");
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    Ok(cmd.spawn()?)
}

/// Decode exactly one RGBA frame (blocking, used for paused scrubbing).
pub fn decode_frame(
    ffmpeg: &Path,
    clip: &PreviewClip,
    w: u32,
    h: u32,
    fps: f64,
    timeline_time: f64,
) -> std::io::Result<Vec<u8>> {
    let src_pos = clip.in_point + (timeline_time - clip.start).max(0.0) * clip.speed;
    let mut child = spawn_oneshot_decoder(ffmpeg, clip, w, h, fps, src_pos)?;
    let mut stdout = child.stdout.take().expect("stdout piped");
    let frame_bytes = (w as usize) * (h as usize) * 4;
    let mut buf = vec![0u8; frame_bytes];
    let result = stdout.read_exact(&mut buf);
    kill_child(&mut child);
    result?;
    Ok(buf)
}

/// Straight-alpha "over" compositing (dst starts transparent).
fn blend_over(dst: &mut [u8], src: &[u8], opacity: f32) {
    let op = opacity.clamp(0.0, 1.0);
    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let sa = (s[3] as f32 / 255.0) * op;
        if sa >= 0.999 {
            d.copy_from_slice(s);
        } else if sa > 0.001 {
            let da = d[3] as f32 / 255.0;
            let out_a = sa + da * (1.0 - sa);
            if out_a > 0.001 {
                for i in 0..3 {
                    let mix = (s[i] as f32 * sa + d[i] as f32 * da * (1.0 - sa)) / out_a;
                    d[i] = mix.round().clamp(0.0, 255.0) as u8;
                }
                d[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// Evaluate an f32 opacity curve at `t` (piecewise linear).
fn curve_at(curve: &[(f32, f32)], t: f32) -> f32 {
    if curve.is_empty() {
        return 1.0;
    }
    if t <= curve[0].0 {
        return curve[0].1;
    }
    let last = curve[curve.len() - 1];
    if t >= last.0 {
        return last.1;
    }
    for w in curve.windows(2) {
        if t >= w[0].0 && t <= w[1].0 {
            let span = (w[1].0 - w[0].0).max(1e-6);
            let f = (t - w[0].0) / span;
            return w[0].1 * (1.0 - f) + w[1].1 * f;
        }
    }
    last.1
}

// ---------------------------------------------------------------------------
// Playback session
// ---------------------------------------------------------------------------

struct LayerDecoder {
    child: Option<Child>,
    reader: Option<BufReader<ChildStdout>>,
    buf: Vec<u8>,
    eof: bool,
    /// Text layers: pre-rasterized RGBA frame (never EOFs).
    text_frame: Option<Vec<u8>>,
    curve: Vec<(f32, f32)>,
    grade: Option<GradeTransform>,
}

impl LayerDecoder {
    fn kill(&mut self) {
        if let Some(child) = self.child.as_mut() {
            kill_child(child);
        }
        self.child = None;
    }
}

impl Drop for LayerDecoder {
    fn drop(&mut self) {
        self.kill();
    }
}

struct Session {
    layers: Vec<LayerDecoder>,
    last_frames: Vec<Vec<u8>>,
    /// Effective timeline start of each layer (for curve evaluation).
    layer_starts: Vec<f64>,
    start_time: f64,
    end_time: f64,
    fps: f64,
    frame_bytes: usize,
    frames_sent: u64,
    started: Instant,
}

impl Session {
    fn new(
        ffmpeg: &Path,
        time: f64,
        video: Option<&PreviewSnapshot>,
        end_time: f64,
    ) -> std::io::Result<Self> {
        let mut layers = Vec::new();
        let mut layer_starts = Vec::new();
        let (frame_bytes, fps) = match video {
            Some(snap) => {
                let fb = snap.width as usize * snap.height as usize * 4;
                for clip in &snap.layers {
                    layer_starts.push(clip.start);
                    if clip.is_text {
                        // Rasterize once at session size.
                        let text_frame = clip
                            .text
                            .as_ref()
                            .map(|t| {
                                crate::textrender::render_text(t, snap.width, snap.height, t.size as f32)
                                    .into_raw()
                            })
                            .unwrap_or_default();
                        layers.push(LayerDecoder {
                            child: None,
                            reader: None,
                            buf: Vec::new(),
                            eof: false,
                            text_frame: Some(text_frame),
                            curve: f32_curve(&clip.opacity_curve),
                            grade: clip.grade.clone(),
                        });
                        continue;
                    }
                    // Start decoding at the requested time (mid-clip starts
                    // no longer replay the clip from its beginning).
                    let seek_local = (time - clip.start).max(0.0);
                    let (child, reader) = spawn_stream_decoder(
                        ffmpeg,
                        clip,
                        snap.width,
                        snap.height,
                        snap.fps,
                        seek_local,
                    )?;
                    layers.push(LayerDecoder {
                        child: Some(child),
                        reader: Some(reader),
                        buf: vec![0u8; fb],
                        eof: false,
                        text_frame: None,
                        curve: f32_curve(&clip.opacity_curve),
                        grade: clip.grade.clone(),
                    });
                }
                (fb, snap.fps)
            }
            None => (0, 30.0),
        };
        Ok(Session {
            layers,
            last_frames: Vec::new(),
            layer_starts,
            start_time: time,
            end_time,
            fps,
            frame_bytes,
            frames_sent: 0,
            started: Instant::now(),
        })
    }

    /// Composite + emit the next frame. Paces itself against wall-clock time.
    fn step(&mut self, tx: &SyncSender<Evt>) -> StepResult {
        // Sleep until this frame's deadline (skip if we are behind).
        let target = self.started + Duration::from_secs_f64(self.frames_sent as f64 / self.fps);
        let now = Instant::now();
        if target > now {
            std::thread::sleep(target - now);
        }
        let time = self.start_time + self.frames_sent as f64 / self.fps;
        if time >= self.end_time {
            return StepResult::Ended;
        }

        // Audio-only playback: advance time without video frames.
        if self.layers.is_empty() {
            self.frames_sent += 1;
            return match tx.send(Evt::Tick { time }) {
                Ok(()) => StepResult::Sent,
                Err(_) => StepResult::Ended,
            };
        }

        let mut composite = vec![0u8; self.frame_bytes];
        let mut any_alive = false;
        for (i, layer) in self.layers.iter_mut().enumerate() {
            let local_t = (time - self.layer_starts.get(i).copied().unwrap_or(self.start_time)) as f32;
            let op = curve_at(&layer.curve, local_t);
            if let Some(text_frame) = &layer.text_frame {
                // Text layer: static rasterized image with curve opacity.
                blend_over(&mut composite, text_frame, op);
                any_alive = true;
                continue;
            }
            if layer.eof {
                if let Some(last) = self.last_frames.get(i) {
                    blend_over(&mut composite, last, op);
                }
                continue;
            }
            let Some(reader) = layer.reader.as_mut() else {
                layer.eof = true;
                continue;
            };
            match reader.read_exact(&mut layer.buf) {
                Ok(()) => {
                    any_alive = true;
                    if self.last_frames.len() <= i {
                        self.last_frames.resize(i + 1, Vec::new());
                    }
                    if let Some(grade) = &layer.grade {
                        grade.apply_rgba(&mut layer.buf);
                    }
                    self.last_frames[i] = layer.buf.clone();
                    blend_over(&mut composite, &layer.buf, op);
                }
                Err(_) => {
                    layer.eof = true;
                    layer.kill();
                }
            }
        }
        self.frames_sent += 1;
        if !any_alive {
            return StepResult::NeedSnapshot;
        }
        match tx.send(Evt::Frame {
            time,
            rgba: Arc::new(composite),
        }) {
            Ok(()) => StepResult::Sent,
            Err(_) => StepResult::Ended, // GUI went away
        }
    }
}

enum StepResult {
    Sent,
    Ended,
    /// All decoders hit EOF while the timeline continues.
    NeedSnapshot,
}

fn f32_curve(curve: &Curve) -> Vec<(f32, f32)> {
    curve.iter().map(|(t, v)| (*t as f32, *v as f32)).collect()
}

// ---------------------------------------------------------------------------
// Engine thread
// ---------------------------------------------------------------------------

pub struct PlaybackEngine {
    cmd_tx: Sender<Cmd>,
    evt_rx: Receiver<Evt>,
}

impl PlaybackEngine {
    pub fn spawn(ffmpeg: PathBuf) -> Self {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Cmd>();
        // Bounded event channel: if the UI ever falls behind (huge frames,
        // busy GPU, minimized window), the engine blocks instead of filling
        // RAM with an unbounded queue of multi-megabyte frames.
        let (evt_tx, evt_rx) = std::sync::mpsc::sync_channel::<Evt>(8);
        std::thread::spawn(move || {
            run_engine(ffmpeg, cmd_rx, evt_tx);
        });
        PlaybackEngine { cmd_tx, evt_rx }
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.cmd_tx.send(cmd);
    }

    /// Non-blocking poll of engine events.
    pub fn try_recv(&mut self) -> Option<Evt> {
        match self.evt_rx.try_recv() {
            Ok(evt) => Some(evt),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
        }
    }
}

impl Drop for PlaybackEngine {
    fn drop(&mut self) {
        self.send(Cmd::Shutdown);
    }
}

fn run_engine(ffmpeg: PathBuf, cmd_rx: Receiver<Cmd>, evt_tx: SyncSender<Evt>) {
    #[derive(PartialEq)]
    enum Mode {
        Idle,
        Playing,
    }
    let mut mode = Mode::Idle;
    let mut session: Option<Session> = None;

    loop {
        match mode {
            Mode::Idle => match cmd_rx.recv() {
                Ok(Cmd::Play {
                    time,
                    video,
                    end_time,
                }) => match Session::new(&ffmpeg, time, video.as_ref(), end_time) {
                    Ok(s) => {
                        session = Some(s);
                        mode = Mode::Playing;
                    }
                    Err(e) => {
                        let _ = evt_tx.send(Evt::Error(format!("preview decoder failed: {e}")));
                    }
                },
                Ok(Cmd::Seek { time, snapshot }) => {
                    if let Some(snap) = snapshot {
                        one_shot(&ffmpeg, time, &snap, &evt_tx);
                    }
                }
                Ok(Cmd::Pause) | Ok(Cmd::Shutdown) => {}
                Err(_) => return,
            },
            Mode::Playing => {
                // Drain pending commands (last one wins).
                let mut cmd = None;
                loop {
                    match cmd_rx.try_recv() {
                        Ok(c) => {
                            cmd = Some(c);
                        }
                        Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
                    }
                }
                if let Some(c) = cmd {
                    match c {
                        Cmd::Pause => {
                            session = None;
                            mode = Mode::Idle;
                            continue;
                        }
                        Cmd::Seek { time, snapshot } => {
                            session = None;
                            mode = Mode::Idle;
                            if let Some(snap) = snapshot {
                                one_shot(&ffmpeg, time, &snap, &evt_tx);
                            }
                            continue;
                        }
                        Cmd::Play {
                            time,
                            video,
                            end_time,
                        } => {
                            session = Session::new(&ffmpeg, time, video.as_ref(), end_time).ok();
                            if session.is_none() {
                                mode = Mode::Idle;
                                let _ = evt_tx.send(Evt::Error("preview decoder failed".into()));
                            }
                            continue;
                        }
                        Cmd::Shutdown => return,
                    }
                }
                let Some(s) = session.as_mut() else {
                    mode = Mode::Idle;
                    continue;
                };
                let outcome = {
                    let current_time = s.start_time + s.frames_sent as f64 / s.fps;
                    let end = s.end_time;
                    match s.step(&evt_tx) {
                        StepResult::Sent => None,
                        StepResult::Ended => Some(Evt::Ended { time: end }),
                        StepResult::NeedSnapshot => {
                            let t = current_time.min(end);
                            if t >= end {
                                Some(Evt::Ended { time: end })
                            } else {
                                Some(Evt::NeedSnapshot { time: t })
                            }
                        }
                    }
                };
                if let Some(evt) = outcome {
                    session = None;
                    mode = Mode::Idle;
                    let _ = evt_tx.send(evt);
                }
            }
        }
    }
}

fn one_shot(ffmpeg: &Path, time: f64, snap: &PreviewSnapshot, evt_tx: &SyncSender<Evt>) {
    let frame_bytes = snap.width as usize * snap.height as usize * 4;
    let mut composite = vec![0u8; frame_bytes];
    let mut ok_any = false;
    for clip in &snap.layers {
        let op = curve_at(&f32_curve(&clip.opacity_curve), (time - clip.start) as f32);
        if clip.is_text {
            if let Some(text) = &clip.text {
                let img = crate::textrender::render_text(text, snap.width, snap.height, text.size as f32);
                if let Some(grade) = &clip.grade {
                    let mut raw = img.into_raw();
                    grade.apply_rgba(&mut raw);
                    blend_over(&mut composite, &raw, op);
                } else {
                    blend_over(&mut composite, img.as_raw(), op);
                }
                ok_any = true;
            }
            continue;
        }
        match decode_frame(ffmpeg, clip, snap.width, snap.height, snap.fps, time) {
            Ok(mut frame) => {
                if let Some(grade) = &clip.grade {
                    grade.apply_rgba(&mut frame);
                }
                ok_any = true;
                blend_over(&mut composite, &frame, op);
            }
            Err(e) => {
                log::debug!("one-shot decode failed for clip {}: {e}", clip.clip_id);
            }
        }
    }
    if ok_any {
        let _ = evt_tx.send(Evt::Frame {
            time,
            rgba: Arc::new(composite),
        });
    } else {
        let _ = evt_tx.send(Evt::Error("could not decode frame (is ffmpeg available?)".into()));
    }
}

/// Convenience for the GUI: ensure even preview size.
pub fn even_preview_size(w: u32, h: u32, scale: f32) -> (u32, u32) {
    let even = |v: f32| (v * scale).round().max(16.0) as u32 / 2 * 2;
    (even(w as f32), even(h as f32))
}
