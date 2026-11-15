//! Frame-accurate preview: composites the active video layers into RGBA
//! frames and streams them to the GUI.
//!
//! Playback uses one streaming `ffmpeg -f rawvideo` decoder per active layer;
//! scrubbing decodes a single frame on demand. Compositing (crop → warp
//! (scale/rotate/flip/position) → grade → blend modes → straight-alpha over,
//! opacity curves, text overlays) happens on the engine thread so the UI
//! never blocks.
//!
//! Preview always decodes the asset's PROXY when one exists (`preview_path`),
//! which is what keeps 4K/8K timelines smooth on ordinary machines.

use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Stdio};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::colorgrade::GradeTransform;
use crate::ffmpeg::{base_command, kill_child};
use crate::project::{BlendMode, Clip, Crop, Curve, Keyframe, Project, TextClipData};
use crate::warp::{blend_over_mode, crop_rgba, warp_into, FrameTransform};

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
    /// Crop fractions.
    pub crop: Crop,
    /// Blend mode over the layers below.
    pub blend: BlendMode,
    /// Static flip flags.
    pub flip_h: bool,
    pub flip_v: bool,
    /// Transform keyframe curves (clip-local seconds → value). Empty when
    /// the property is not animated.
    pub scale_keys: Curve,
    pub pos_x_keys: Curve,
    pub pos_y_keys: Curve,
    pub rot_keys: Curve,
    /// Static fallback values used when the matching curve is empty.
    pub base_scale: f64,
    pub base_pos_x: f64,
    pub base_pos_y: f64,
    pub base_rot: f64,
    pub base_pivot_x: f64,
    pub base_pivot_y: f64,
}

impl PreviewClip {
    /// Frame transform at clip-local time `t`.
    pub fn transform_at(&self, t: f64) -> FrameTransform {
        FrameTransform {
            scale_x: curve_val(&self.scale_keys, self.base_scale, t).max(0.01) as f32,
            scale_y: curve_val(&self.scale_keys, self.base_scale, t).max(0.01) as f32,
            pos_x: curve_val(&self.pos_x_keys, self.base_pos_x, t) as f32,
            pos_y: curve_val(&self.pos_y_keys, self.base_pos_y, t) as f32,
            rotation: curve_val(&self.rot_keys, self.base_rot, t) as f32,
            pivot_x: self.base_pivot_x as f32,
            pivot_y: self.base_pivot_y as f32,
            flip_h: self.flip_h,
            flip_v: self.flip_v,
        }
    }

}

fn curve_val(curve: &Curve, base: f64, t: f64) -> f64 {
    if curve.is_empty() {
        return base;
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
            let span = (w[1].0 - w[0].0).max(1e-9);
            let f = (t - w[0].0) / span;
            return w[0].1 * (1.0 - f) + w[1].1 * f;
        }
    }
    last.1
}

fn prop_curve(clip: &Clip, pick: impl Fn(&Keyframe) -> Option<f64>) -> Curve {
    clip.keyframes
        .iter()
        .filter_map(|k| pick(k).map(|v| (k.t, v)))
        .collect()
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
/// memory bandwidth (33 MB/frame) and can exceed GPU texture limits. Exports
/// are unaffected: they run through ffmpeg at full project resolution.
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
                    crop: Default::default(),
                    blend: BlendMode::Normal,
                    flip_h: false,
                    flip_v: false,
                    scale_keys: Vec::new(),
                    pos_x_keys: Vec::new(),
                    pos_y_keys: Vec::new(),
                    rot_keys: Vec::new(),
                    base_scale: 1.0,
                    base_pos_x: 0.0,
                    base_pos_y: 0.0,
                    base_rot: 0.0,
                    base_pivot_x: 0.5,
                    base_pivot_y: 0.5,
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
            // Transform curves are stored in the CLIP's local time; the
            // pull-in shifts the layer start, so evaluate via (time - start).
            layers.push(PreviewClip {
                clip_id: clip.id,
                // Proxy-aware: preview decodes the optimized stand-in.
                asset_path: asset.preview_path().to_path_buf(),
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
                crop: clip.crop,
                blend: clip.blend,
                flip_h: clip.transform.flip_h,
                flip_v: clip.transform.flip_v,
                scale_keys: prop_curve(clip, |k| k.scale_x),
                pos_x_keys: prop_curve(clip, |k| k.pos_x),
                pos_y_keys: prop_curve(clip, |k| k.pos_y),
                rot_keys: prop_curve(clip, |k| k.rotation),
                base_scale: clip.transform.scale_x.max(clip.transform.scale_y.max(1.0e-6)),
                base_pos_x: clip.transform.pos_x,
                base_pos_y: clip.transform.pos_y,
                base_rot: clip.transform.rotation,
                base_pivot_x: clip.transform.pivot_x,
                base_pivot_y: clip.transform.pivot_y,
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
// Layer compositing
// ---------------------------------------------------------------------------

/// Post-process one decoded layer frame (grade → crop → warp into
/// `canvas`) and blend it over `composite`. Shared by live frames and the
/// frozen "last frame" used after a layer hits EOF.
fn composite_layer(
    layer: &LayerDecoder,
    canvas: &mut [u8],
    raw: &mut Vec<u8>,
    composite: &mut [u8],
    ow: u32,
    oh: u32,
    local_t: f64,
    op: f32,
) {
    // 1. Color grade (matrix + wheels + LUT).
    if let Some(grade) = &layer.grade {
        grade.apply_rgba(raw);
    }
    // 2. Crop.
    let (cw, ch) = if layer.crop.is_identity() || !layer.crop.is_valid() {
        (ow, oh)
    } else {
        crop_rgba(raw, ow, oh, layer.crop)
    };
    // 3. Warp (scale / rotate / flip / position) into a full-size canvas.
    let t = layer.transform.at(local_t);
    if t.is_identity() && cw == ow && ch == oh {
        // Fast path: straight alpha-over.
        blend_over_mode(composite, raw, layer.blend, op);
        return;
    }
    canvas.iter_mut().for_each(|b| *b = 0);
    warp_into(canvas, ow, oh, raw, cw, ch, t);
    blend_over_mode(composite, canvas, layer.blend, op);
}

/// Per-layer transform data (curves + static base) resolved once per session.
#[derive(Debug, Clone, Default)]
struct LayerTransform {
    scale: Curve,
    pos_x: Curve,
    pos_y: Curve,
    rot: Curve,
    base_scale: f64,
    base_pos_x: f64,
    base_pos_y: f64,
    base_rot: f64,
    base_pivot_x: f64,
    base_pivot_y: f64,
    flip_h: bool,
    flip_v: bool,
}

impl LayerTransform {
    fn at(&self, t: f64) -> FrameTransform {
        FrameTransform {
            scale_x: curve_val(&self.scale, self.base_scale, t).max(0.01) as f32,
            scale_y: curve_val(&self.scale, self.base_scale, t).max(0.01) as f32,
            pos_x: curve_val(&self.pos_x, self.base_pos_x, t) as f32,
            pos_y: curve_val(&self.pos_y, self.base_pos_y, t) as f32,
            rotation: curve_val(&self.rot, self.base_rot, t) as f32,
            pivot_x: self.base_pivot_x as f32,
            pivot_y: self.base_pivot_y as f32,
            flip_h: self.flip_h,
            flip_v: self.flip_v,
        }
    }

    fn is_animated(&self) -> bool {
        !self.scale.is_empty() || !self.pos_x.is_empty() || !self.pos_y.is_empty() || !self.rot.is_empty()
    }
}

fn layer_transform_from(clip: &PreviewClip) -> LayerTransform {
    LayerTransform {
        scale: clip.scale_keys.clone(),
        pos_x: clip.pos_x_keys.clone(),
        pos_y: clip.pos_y_keys.clone(),
        rot: clip.rot_keys.clone(),
        base_scale: clip.base_scale,
        base_pos_x: clip.base_pos_x,
        base_pos_y: clip.base_pos_y,
        base_rot: clip.base_rot,
        base_pivot_x: clip.base_pivot_x,
        base_pivot_y: clip.base_pivot_y,
        flip_h: clip.flip_h,
        flip_v: clip.flip_v,
    }
}

struct LayerDecoder {
    child: Option<Child>,
    reader: Option<BufReader<ChildStdout>>,
    buf: Vec<u8>,
    eof: bool,
    /// Text layers: pre-rasterized RGBA frame (never EOFs).
    text_frame: Option<Vec<u8>>,
    curve: Vec<(f32, f32)>,
    grade: Option<GradeTransform>,
    crop: Crop,
    blend: BlendMode,
    transform: LayerTransform,
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
    /// Per-layer warp canvas (full frame size).
    canvases: Vec<Vec<u8>>,
    /// Effective timeline start of each layer (for curve evaluation).
    layer_starts: Vec<f64>,
    start_time: f64,
    end_time: f64,
    fps: f64,
    frame_bytes: usize,
    width: u32,
    height: u32,
    frames_sent: u64,
    started: Instant,
}

impl Session {
    #[allow(clippy::too_many_arguments)]
    fn new(
        ffmpeg: &Path,
        time: f64,
        video: Option<&PreviewSnapshot>,
        end_time: f64,
    ) -> std::io::Result<Self> {
        let mut layers = Vec::new();
        let mut layer_starts = Vec::new();
        let (frame_bytes, fps, width, height) = match video {
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
                            crop: Default::default(),
                            blend: clip.blend,
                            transform: LayerTransform::default(),
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
                        crop: clip.crop,
                        blend: clip.blend,
                        transform: layer_transform_from(clip),
                    });
                }
                (fb, snap.fps, snap.width, snap.height)
            }
            None => (0, 30.0, 0, 0),
        };
        Ok(Session {
            canvases: layers.iter().map(|_| vec![0u8; frame_bytes]).collect(),
            layers,
            last_frames: Vec::new(),
            layer_starts,
            start_time: time,
            end_time,
            fps,
            frame_bytes,
            width,
            height,
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
        let (ow, oh) = (self.width, self.height);
        for (i, layer) in self.layers.iter_mut().enumerate() {
            let local_t = (time - self.layer_starts.get(i).copied().unwrap_or(self.start_time)) as f32;
            let op = curve_at(&layer.curve, local_t);
            if let Some(text_frame) = &layer.text_frame {
                // Text layer: static rasterized image with curve opacity.
                blend_over_mode(&mut composite, text_frame, layer.blend, op);
                any_alive = true;
                continue;
            }
            let lt = local_t as f64;
            let needs_canvas = layer.blend != BlendMode::Normal
                || !layer.crop.is_identity()
                || layer.transform.is_animated()
                || layer.transform.at(0.0) != FrameTransform::identity();
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
                    if needs_canvas {
                        let mut raw = layer.buf.clone();
                        let canvas = self.canvases[i].as_mut_slice();
                        let layer_ref = &*layer;
                        composite_layer(layer_ref, canvas, &mut raw, &mut composite, ow, oh, lt, op);
                        // Keep the PRE-GRADE frame as the frozen fallback so
                        // grading is not applied twice after EOF.
                        self.last_frames[i] = layer.buf.clone();
                    } else {
                        // Store the PRE-grade frame (composite_layer grades it
                        // again for the EOF freeze path).
                        self.last_frames[i] = layer.buf.clone();
                        if let Some(grade) = &layer.grade {
                            grade.apply_rgba(&mut layer.buf);
                        }
                        blend_over_mode(&mut composite, &layer.buf, layer.blend, op);
                    }
                }
                Err(_) => {
                    layer.eof = true;
                    layer.kill();
                    // Freeze on the last frame (already graded, pre-warp).
                    if let Some(last) = self.last_frames.get(i).cloned() {
                        let mut raw = last;
                        let canvas = self.canvases[i].as_mut_slice();
                        let layer_ref = &*layer;
                        composite_layer(layer_ref, canvas, &mut raw, &mut composite, ow, oh, lt, op);
                    }
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
        std::thread::Builder::new()
            .name("vedit-engine".into())
            .spawn(move || {
                // A panic here must never take the app down (release builds
                // used to abort on it): catch, report, and keep the loop idle.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_engine(ffmpeg, cmd_rx, evt_tx)
                }));
                if result.is_err() {
                    log::error!("preview engine panicked; preview disabled until restart");
                }
            })
            .expect("spawn engine thread");
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
    let mut canvas = vec![0u8; frame_bytes];
    for clip in &snap.layers {
        let op = curve_at(&f32_curve(&clip.opacity_curve), (time - clip.start) as f32);
        if clip.is_text {
            if let Some(text) = &clip.text {
                let img = crate::textrender::render_text(text, snap.width, snap.height, text.size as f32);
                let mut raw = img.into_raw();
                if let Some(grade) = &clip.grade {
                    grade.apply_rgba(&mut raw);
                }
                blend_over_mode(&mut composite, &raw, clip.blend, op);
                ok_any = true;
            }
            continue;
        }
        match decode_frame(ffmpeg, clip, snap.width, snap.height, snap.fps, time) {
            Ok(mut frame) => {
                // grade → crop → warp → blend
                if let Some(grade) = &clip.grade {
                    grade.apply_rgba(&mut frame);
                }
                let (cw, ch) = if clip.crop.is_identity() || !clip.crop.is_valid() {
                    (snap.width, snap.height)
                } else {
                    crop_rgba(&mut frame, snap.width, snap.height, clip.crop)
                };
                let t = clip.transform_at(time - clip.start);
                if t.is_identity() && cw == snap.width && ch == snap.height {
                    blend_over_mode(&mut composite, &frame, clip.blend, op);
                } else {
                    canvas.iter_mut().for_each(|b| *b = 0);
                    warp_into(&mut canvas, snap.width, snap.height, &frame, cw, ch, t);
                    blend_over_mode(&mut composite, &canvas, clip.blend, op);
                }
                ok_any = true;
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
