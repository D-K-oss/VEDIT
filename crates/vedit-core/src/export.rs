//! Export pipeline: turns the project into a single ffmpeg command.
//!
//! Design: every clip instance becomes one `-ss/-t -i` input. All video
//! layers composite onto a black background via `overlay` chains; all audio
//! mixes via `amix`. This gives Premiere-like layering while staying a
//! one-shot ffmpeg render.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::error::{ensure_parent_dir, Result, VeditError};
use crate::ffmpeg::{base_command, kill_child, secs, FfmpegLocator};
use crate::media::MediaKind;
use crate::project::{Project, TrackKind};
use crate::timeline;

// ---------------------------------------------------------------------------
// Settings model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RateMode {
    /// Constant-ish average bitrate, in kbps.
    Bitrate { kbps: u32 },
    /// Quality-targeted (CRF) for codecs that support it.
    Crf { crf: u8 },
}

#[derive(Debug, Clone)]
pub struct VideoSettings {
    pub codec: &'static str,
    pub rate: RateMode,
    /// Encoder speed preset (e.g. "medium"); None for codecs without one.
    pub preset: Option<&'static str>,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct AudioSettings {
    pub codec: &'static str,
    pub bitrate_kbps: u32,
    pub sample_rate: u32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResChoice {
    Project,
    Fixed(u32, u32),
    /// Width fixed, height derived from project aspect.
    FitWidth(u32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FpsChoice {
    Project,
    Fixed(f64),
}

#[derive(Debug, Clone)]
pub struct ExportSettings {
    pub output: PathBuf,
    pub container: &'static str,
    pub video: VideoSettings,
    pub audio: AudioSettings,
    pub res: ResChoice,
    pub fps: FpsChoice,
    /// Extra args appended verbatim (power users).
    pub extra_args: String,
}

impl Default for ExportSettings {
    fn default() -> Self {
        ExportSettings {
            output: PathBuf::from("export.mp4"),
            container: "mp4",
            video: VideoSettings {
                codec: "libx264",
                rate: RateMode::Crf { crf: 18 },
                preset: Some("medium"),
                enabled: true,
            },
            audio: AudioSettings {
                codec: "aac",
                bitrate_kbps: 192,
                sample_rate: 48_000,
                enabled: true,
            },
            res: ResChoice::Project,
            fps: FpsChoice::Project,
            extra_args: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Codec tables
// ---------------------------------------------------------------------------

pub struct VideoCodecDef {
    pub id: &'static str,
    pub label: &'static str,
    pub supports_crf: bool,
    pub presets: &'static [&'static str],
    pub pix_fmt: Option<&'static str>,
    pub lossless: bool,
    /// Hardware encoder (NVENC / QSV / AMF) — availability depends on GPU.
    pub hw: bool,
    /// Extra encoder args always applied.
    pub extra: &'static [(&'static str, &'static str)],
    /// Extra args required when CRF mode is used (VP9 needs -b:v 0).
    pub crf_extra: &'static [(&'static str, &'static str)],
    /// Sensible default bitrate (kbps) when bitrate mode is selected.
    pub default_bitrate_kbps: u32,
}

pub const VIDEO_CODECS: &[VideoCodecDef] = &[
    VideoCodecDef {
        id: "libx264",
        label: "H.264 / AVC (libx264)",
        supports_crf: true,
        presets: &["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow"],
        pix_fmt: Some("yuv420p"),
        lossless: false,
        hw: false,
        extra: &[],
        crf_extra: &[],
        default_bitrate_kbps: 12_000,
    },
    VideoCodecDef {
        id: "h264_nvenc",
        label: "H.264 · NVENC (NVIDIA GPU)",
        supports_crf: false,
        presets: &["default", "p1", "p2", "p3", "p4", "p5", "p6", "p7"],
        pix_fmt: Some("yuv420p"),
        lossless: false,
        hw: true,
        extra: &[],
        crf_extra: &[],
        default_bitrate_kbps: 12_000,
    },
    VideoCodecDef {
        id: "h264_qsv",
        label: "H.264 · Quick Sync (Intel GPU)",
        supports_crf: false,
        presets: &["veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow"],
        pix_fmt: Some("yuv420p"),
        lossless: false,
        hw: true,
        extra: &[],
        crf_extra: &[],
        default_bitrate_kbps: 12_000,
    },
    VideoCodecDef {
        id: "h264_amf",
        label: "H.264 · AMF (AMD GPU)",
        supports_crf: false,
        presets: &["speed", "balanced", "quality"],
        pix_fmt: Some("yuv420p"),
        lossless: false,
        hw: true,
        extra: &[],
        crf_extra: &[],
        default_bitrate_kbps: 12_000,
    },
    VideoCodecDef {
        id: "libx265",
        label: "H.265 / HEVC (libx265)",
        supports_crf: true,
        presets: &["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow"],
        pix_fmt: Some("yuv420p"),
        lossless: false,
        hw: false,
        extra: &[("-tag:v", "hvc1")],
        crf_extra: &[],
        default_bitrate_kbps: 8_000,
    },
    VideoCodecDef {
        id: "hevc_nvenc",
        label: "H.265 · NVENC (NVIDIA GPU)",
        supports_crf: false,
        presets: &["default", "p1", "p2", "p3", "p4", "p5", "p6", "p7"],
        pix_fmt: Some("yuv420p"),
        lossless: false,
        hw: true,
        extra: &[("-tag:v", "hvc1")],
        crf_extra: &[],
        default_bitrate_kbps: 8_000,
    },
    VideoCodecDef {
        id: "hevc_qsv",
        label: "H.265 · Quick Sync (Intel GPU)",
        supports_crf: false,
        presets: &["veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow"],
        pix_fmt: Some("yuv420p"),
        lossless: false,
        hw: true,
        extra: &[("-tag:v", "hvc1")],
        crf_extra: &[],
        default_bitrate_kbps: 8_000,
    },
    VideoCodecDef {
        id: "hevc_amf",
        label: "H.265 · AMF (AMD GPU)",
        supports_crf: false,
        presets: &["speed", "balanced", "quality"],
        pix_fmt: Some("yuv420p"),
        lossless: false,
        hw: true,
        extra: &[("-tag:v", "hvc1")],
        crf_extra: &[],
        default_bitrate_kbps: 8_000,
    },
    VideoCodecDef {
        id: "libvpx-vp9",
        label: "VP9 (libvpx-vp9)",
        supports_crf: true,
        presets: &[],
        pix_fmt: Some("yuv420p"),
        lossless: false,
        hw: false,
        extra: &[("-deadline", "good"), ("-cpu-used", "2"), ("-row-mt", "1")],
        crf_extra: &[("-b:v", "0")],
        default_bitrate_kbps: 6_000,
    },
    VideoCodecDef {
        id: "libsvtav1",
        label: "AV1 (SVT-AV1)",
        supports_crf: true,
        presets: &["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12"],
        pix_fmt: Some("yuv420p"),
        lossless: false,
        hw: false,
        extra: &[],
        crf_extra: &[],
        default_bitrate_kbps: 6_000,
    },
    VideoCodecDef {
        id: "prores_ks",
        label: "Apple ProRes 422 (prores_ks)",
        supports_crf: false,
        presets: &[],
        pix_fmt: Some("yuv422p10le"),
        lossless: false,
        hw: false,
        extra: &[("-profile:v", "2"), ("-vendor", "apl0")],
        crf_extra: &[],
        default_bitrate_kbps: 150_000,
    },
    VideoCodecDef {
        id: "libvpx",
        label: "VP8 (libvpx)",
        supports_crf: false,
        presets: &[],
        pix_fmt: Some("yuv420p"),
        lossless: false,
        hw: false,
        extra: &[("-deadline", "good"), ("-cpu-used", "2")],
        crf_extra: &[],
        default_bitrate_kbps: 5_000,
    },
    VideoCodecDef {
        id: "mpeg4",
        label: "MPEG-4 Part 2 (mpeg4)",
        supports_crf: false,
        presets: &[],
        pix_fmt: Some("yuv420p"),
        lossless: false,
        hw: false,
        extra: &[],
        crf_extra: &[],
        default_bitrate_kbps: 8_000,
    },
    VideoCodecDef {
        id: "ffv1",
        label: "FFV1 (mathematically lossless)",
        supports_crf: false,
        presets: &[],
        pix_fmt: Some("yuv420p10le"),
        lossless: true,
        hw: false,
        extra: &[("-level", "3"), ("-coder", "1"), ("-g", "1")],
        crf_extra: &[],
        default_bitrate_kbps: 0,
    },
    VideoCodecDef {
        id: "gif",
        label: "GIF animation",
        supports_crf: false,
        presets: &[],
        pix_fmt: None,
        lossless: false,
        hw: false,
        extra: &[],
        crf_extra: &[],
        default_bitrate_kbps: 0,
    },
];

/// Encoder IDs advertised by this ffmpeg build (parsed from
/// `ffmpeg -encoders`). Used to mark hardware codecs as available.
pub fn available_encoders(locator: &crate::ffmpeg::FfmpegLocator) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    let mut cmd = crate::ffmpeg::base_command(locator.ffmpeg());
    cmd.args(["-hide_banner", "-encoders"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Ok(out) = cmd.output() {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                // Format: " A..... name  description"
                let bytes = line.as_bytes();
                if bytes.len() > 6 && (bytes[0] == b' ' || bytes[0] == b'A' || bytes[0] == b'V') {
                    if let Some(name) = line.get(8..).and_then(|rest| {
                        rest.split_whitespace().next()
                    }) {
                        set.insert(name.to_string());
                    }
                }
            }
        }
    }
    set
}

pub struct AudioCodecDef {
    pub id: &'static str,
    pub label: &'static str,
    pub lossless: bool,
}

pub const AUDIO_CODECS: &[AudioCodecDef] = &[
    AudioCodecDef { id: "aac", label: "AAC", lossless: false },
    AudioCodecDef { id: "libmp3lame", label: "MP3 (libmp3lame)", lossless: false },
    AudioCodecDef { id: "libopus", label: "Opus (libopus)", lossless: false },
    AudioCodecDef { id: "libvorbis", label: "Vorbis (libvorbis)", lossless: false },
    AudioCodecDef { id: "flac", label: "FLAC (lossless)", lossless: true },
    AudioCodecDef { id: "pcm_s16le", label: "WAV PCM 16-bit (lossless)", lossless: true },
];

pub const CONTAINERS: &[&str] = &[
    "mp4", "mkv", "mov", "webm", "avi", "mpg", "ts", "ogv", "gif", "wav", "mp3", "flac", "ogg", "m4a",
];

pub fn is_audio_container(container: &str) -> bool {
    matches!(container, "wav" | "mp3" | "flac" | "ogg" | "m4a")
}

pub fn is_video_container(container: &str) -> bool {
    !is_audio_container(container)
}

pub fn video_codec_def(id: &str) -> Option<&'static VideoCodecDef> {
    VIDEO_CODECS.iter().find(|d| d.id == id)
}

pub fn audio_codec_def(id: &str) -> Option<&'static AudioCodecDef> {
    AUDIO_CODECS.iter().find(|d| d.id == id)
}

// ---------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------

pub struct ExportPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub apply: fn(&mut ExportSettings),
}

pub const PRESETS: &[ExportPreset] = &[
    ExportPreset {
        id: "h264",
        label: "MP4 · H.264 + AAC (recommended)",
        apply: |s| {
            s.container = "mp4";
            s.video = VideoSettings {
                codec: "libx264",
                rate: RateMode::Crf { crf: 18 },
                preset: Some("medium"),
                enabled: true,
            };
            s.audio = AudioSettings { codec: "aac", bitrate_kbps: 192, sample_rate: 48_000, enabled: true };
        },
    },
    ExportPreset {
        id: "h264_hw",
        label: "MP4 · H.264 hardware encode (NVENC/QSV/AMF)",
        apply: |s| {
            s.container = "mp4";
            s.video = VideoSettings {
                codec: "h264_nvenc",
                rate: RateMode::Bitrate { kbps: 12_000 },
                preset: Some("p4"),
                enabled: true,
            };
            s.audio = AudioSettings { codec: "aac", bitrate_kbps: 192, sample_rate: 48_000, enabled: true };
        },
    },
    ExportPreset {
        id: "h265",
        label: "MP4 · H.265/HEVC + AAC (small files)",
        apply: |s| {
            s.container = "mp4";
            s.video = VideoSettings {
                codec: "libx265",
                rate: RateMode::Crf { crf: 24 },
                preset: Some("medium"),
                enabled: true,
            };
            s.audio = AudioSettings { codec: "aac", bitrate_kbps: 160, sample_rate: 48_000, enabled: true };
        },
    },
    ExportPreset {
        id: "vp9",
        label: "WebM · VP9 + Opus (web)",
        apply: |s| {
            s.container = "webm";
            s.video = VideoSettings {
                codec: "libvpx-vp9",
                rate: RateMode::Crf { crf: 32 },
                preset: None,
                enabled: true,
            };
            s.audio = AudioSettings { codec: "libopus", bitrate_kbps: 160, sample_rate: 48_000, enabled: true };
        },
    },
    ExportPreset {
        id: "av1",
        label: "MP4 · AV1 + Opus (next-gen)",
        apply: |s| {
            s.container = "mp4";
            s.video = VideoSettings {
                codec: "libsvtav1",
                rate: RateMode::Crf { crf: 30 },
                preset: Some("8"),
                enabled: true,
            };
            s.audio = AudioSettings { codec: "libopus", bitrate_kbps: 160, sample_rate: 48_000, enabled: true };
        },
    },
    ExportPreset {
        id: "prores",
        label: "MOV · ProRes 422 + PCM (editing master)",
        apply: |s| {
            s.container = "mov";
            s.video = VideoSettings {
                codec: "prores_ks",
                rate: RateMode::Bitrate { kbps: 150_000 },
                preset: None,
                enabled: true,
            };
            s.audio = AudioSettings { codec: "pcm_s16le", bitrate_kbps: 1536, sample_rate: 48_000, enabled: true };
        },
    },
    ExportPreset {
        id: "lossless",
        label: "MKV · FFV1 + FLAC (lossless archive)",
        apply: |s| {
            s.container = "mkv";
            s.video = VideoSettings {
                codec: "ffv1",
                rate: RateMode::Bitrate { kbps: 0 },
                preset: None,
                enabled: true,
            };
            s.audio = AudioSettings { codec: "flac", bitrate_kbps: 0, sample_rate: 48_000, enabled: true };
        },
    },
    ExportPreset {
        id: "gif",
        label: "GIF · animation (640px, 15fps)",
        apply: |s| {
            s.container = "gif";
            s.video = VideoSettings {
                codec: "gif",
                rate: RateMode::Bitrate { kbps: 0 },
                preset: None,
                enabled: true,
            };
            s.audio = AudioSettings { codec: "aac", bitrate_kbps: 192, sample_rate: 48_000, enabled: false };
            s.res = ResChoice::FitWidth(640);
            s.fps = FpsChoice::Fixed(15.0);
        },
    },
    ExportPreset {
        id: "mp3",
        label: "MP3 · audio only 320 kbps",
        apply: |s| {
            s.container = "mp3";
            s.video.enabled = false;
            s.audio = AudioSettings { codec: "libmp3lame", bitrate_kbps: 320, sample_rate: 48_000, enabled: true };
        },
    },
    ExportPreset {
        id: "wav",
        label: "WAV · audio only (PCM lossless)",
        apply: |s| {
            s.container = "wav";
            s.video.enabled = false;
            s.audio = AudioSettings { codec: "pcm_s16le", bitrate_kbps: 0, sample_rate: 48_000, enabled: true };
        },
    },
];

// ---------------------------------------------------------------------------
// Filter graph + command builder
// ---------------------------------------------------------------------------

/// atempo chain that keeps every factor inside [0.5, 2.0].
fn atempo_chain(speed: f64) -> String {
    if (speed - 1.0).abs() < 1e-6 {
        return String::new();
    }
    let mut factors: Vec<f64> = Vec::new();
    let mut s = speed;
    while s > 2.0 + 1e-9 {
        factors.push(2.0);
        s /= 2.0;
    }
    while s < 0.5 - 1e-9 {
        factors.push(0.5);
        s *= 2.0;
    }
    factors.push(s);
    let mut out = String::new();
    for f in factors {
        out.push_str(&format!(",atempo={:.4}", f));
    }
    out
}

struct GraphCtx {
    parts: Vec<String>,
    audio_labels: Vec<String>,
    /// Label of the current composited video stream ("bg" at first).
    current: String,
}

/// Push one input; returns its index.
fn push_input(args: &mut Vec<String>, path: &std::path::Path, pre: &[&str]) -> usize {
    for a in pre {
        args.push(a.to_string());
    }
    args.push("-i".into());
    args.push(path.display().to_string());
    // index = position among -i flags → count them
    args.iter().enumerate().filter(|(_, a)| a.as_str() == "-i").count() - 1
}

/// Decompose an opacity curve into (base, fade-in, fade-out-start, fade-out-len)
/// or `None` when the curve is genuinely keyframed (needs per-frame alpha).
fn decompose_opacity(curve: &crate::project::Curve, dur: f64) -> Option<(f64, Option<f64>, Option<f64>, Option<f64>)> {
    if curve.is_empty() {
        return Some((1.0, None, None, None));
    }
    // Constant?
    let v0 = curve[0].1;
    if curve.iter().all(|p| (p.1 - v0).abs() < 2e-3) {
        return Some((v0, None, None, None));
    }
    // Plateau base: the value the curve settles on between fades.
    let base = curve.iter().cloned().fold(0.0f64, |a, p| a.max(p.1));
    if base <= 1e-3 {
        return None;
    }
    let mut fade_in: Option<f64> = None;
    let mut fade_out: Option<(f64, f64)> = None; // (start, len)
    for w in curve.windows(2) {
        let (t0, v0) = w[0];
        let (t1, v1) = w[1];
        let n0 = v0 / base;
        let n1 = v1 / base;
        let is_ramp = |a: f64, b: f64| {
            (a - 1.0).abs() < 2e-3 && b.abs() < 2e-3 || a.abs() < 2e-3 && (b - 1.0).abs() < 2e-3
        };
        if is_ramp(n0, n1) {
            if n1 > n0 {
                // 0 → 1 (fade in)
                if t0.abs() > 1e-3 || fade_in.is_some() {
                    return None;
                }
                fade_in = Some(t1 - t0);
            } else {
                // 1 → 0 (fade out)
                if fade_out.is_some() || (t1 - dur).abs() > 2e-2 {
                    return None;
                }
                fade_out = Some((t0, t1 - t0));
            }
        } else if (n0 - n1).abs() > 2e-3 {
            // Non-fade, non-flat segment → keyframed.
            return None;
        }
    }
    Some((base, fade_in, fade_out.map(|f| f.0), fade_out.map(|f| f.1)))
}

/// Build a nested if(lt(...)) expression for a piecewise-linear curve.
/// `var` is the time variable ("T" for geq, "t" for volume).
fn curve_expression(curve: &crate::project::Curve, var: &str) -> String {
    let mut expr = curve
        .last()
        .map(|p| format!("{:.4}", p.1))
        .unwrap_or_else(|| "1.0".to_string());
    for w in curve.windows(2).rev() {
        let (t0, v0) = w[0];
        let (t1, _v1) = w[1];
        let slope = (_v1 - v0) / (t1 - t0).max(1e-9);
        expr = format!(
            "if(lt({var},{t1:.4}),{v0:.4}+({slope:.6})*({var}-{t0:.4}),{expr})",
            var = var,
            t1 = t1,
            v0 = v0,
            slope = slope,
            t0 = t0,
            expr = expr,
        );
    }
    expr
}

/// ffmpeg grade filters for a clip, tolerating a missing/unreadable LUT
/// (warn + skip the grade instead of failing the whole export).
fn grade_filters_lenient(grade: Option<&crate::project::ColorGrade>) -> String {
    match grade {
        None => String::new(),
        Some(g) => match crate::colorgrade::ffmpeg_grade_filters(g) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("color grade skipped: {e}");
                String::new()
            }
        },
    }
}

struct VideoLayerSpec<'a> {
    asset_path: &'a std::path::Path,
    /// Pre-rendered full-frame RGBA overlay (text/title clips).
    text_png: Option<&'a std::path::Path>,
    is_image: bool,
    in_point: f64,
    source_dur: f64,
    start: f64,
    duration: f64,
    opacity_curve: crate::project::Curve,
    grade_filters: String,
    w: u32,
    h: u32,
    fps_s: &'a str,
    /// Crop fractions (applied after fit-to-frame).
    crop: crate::project::Crop,
    /// Blend mode over the composite.
    blend: crate::project::BlendMode,
    /// Static transform (fallback for unkeyframed properties).
    transform: crate::project::ClipTransform,
    /// Transform keyframe curves (clip-local seconds). Empty = static.
    scale_keys: crate::project::Curve,
    pos_x_keys: crate::project::Curve,
    pos_y_keys: crate::project::Curve,
    rot_keys: crate::project::Curve,
}

/// Piecewise-linear curve → ffmpeg expression in variable `var`.
fn transform_expr(curve: &crate::project::Curve, static_value: f64, var: &str) -> String {
    if curve.is_empty() {
        return format!("{:.6}", static_value);
    }
    let mut expr = curve
        .last()
        .map(|p| format!("{:.6}", p.1))
        .unwrap_or_else(|| format!("{:.6}", static_value));
    for w in curve.windows(2).rev() {
        let (t0, v0) = w[0];
        let (t1, _v1) = w[1];
        let slope = (_v1 - v0) / (t1 - t0).max(1e-9);
        expr = format!(
            "if(lt({var},{t1:.4}),{v0:.6}+({slope:.6})*({var}-{t0:.4}),{expr})",
            var = var,
            t1 = t1,
            v0 = v0,
            slope = slope,
            t0 = t0,
            expr = expr,
        );
    }
    expr
}

fn add_video_layer(
    g: &mut GraphCtx,
    args: &mut Vec<String>,
    spec: &VideoLayerSpec<'_>,
) {
    let index = if spec.text_png.is_some() {
        push_input(args, spec.asset_path, &[
            "-loop", "1", "-t", &secs(spec.duration),
        ])
    } else if spec.is_image {
        push_input(args, spec.asset_path, &["-loop", "1", "-t", &secs(spec.duration)])
    } else {
        push_input(args, spec.asset_path, &["-ss", &secs(spec.in_point), "-t", &secs(spec.source_dur)])
    };

    let label = format!("v{index}");
    let dur = spec.duration.max(0.05);
    let mut chain = if spec.text_png.is_some() {
        // Text overlay PNGs are pre-rendered at the exact output size.
        format!("[{index}:v]format=rgba", index = index)
    } else {
        format!(
            "[{index}:v]fps={fps},scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,format=rgba",
            index = index,
            fps = spec.fps_s,
            w = spec.w,
            h = spec.h,
        )
    };

    // ---- crop (fractions of the fitted frame) --------------------------
    let crop = spec.crop;
    if !crop.is_identity() && crop.is_valid() {
        let cw = (1.0 - crop.left - crop.right).clamp(0.05, 1.0);
        let ch = (1.0 - crop.top - crop.bottom).clamp(0.05, 1.0);
        chain.push_str(&format!(
            ",crop=w='iw*{cw:.4}':h='ih*{ch:.4}':x='iw*{l:.4}':y='ih*{t:.4}'",
            cw = cw,
            ch = ch,
            l = crop.left.clamp(0.0, 0.95),
            t = crop.top.clamp(0.0, 0.95),
        ));
    }

    // ---- transform: flip → rotate → scale -------------------------------
    let tr = &spec.transform;
    if tr.flip_h {
        chain.push_str(",hflip");
    }
    if tr.flip_v {
        chain.push_str(",vflip");
    }
    if !spec.rot_keys.is_empty() {
        // Per-frame rotation (angle expression, transparent corners).
        let expr = transform_expr(&spec.rot_keys, tr.rotation, "t");
        chain.push_str(&format!(",rotate='{expr}*PI/180':c=black@0:ow=iw:oh=ih"));
    } else if tr.rotation.abs() > 1e-4 {
        chain.push_str(&format!(
            ",rotate='{:.6}*PI/180':c=black@0:ow=iw:oh=ih",
            tr.rotation
        ));
    }
    if !spec.scale_keys.is_empty() {
        let expr = transform_expr(&spec.scale_keys, tr.scale_x, "t");
        chain.push_str(&format!(
            ",scale=w='trunc(iw*max({e},0.01)/2)*2':h='trunc(ih*max({e},0.01)/2)*2':eval=frame",
            e = expr
        ));
    } else if (tr.scale_x - 1.0).abs() > 1e-4 || (tr.scale_y - 1.0).abs() > 1e-4 {
        chain.push_str(&format!(
            ",scale=iw*{sx:.4}:ih*{sy:.4}",
            sx = tr.scale_x.max(0.01),
            sy = tr.scale_y.max(0.01),
        ));
    }

    // Opacity: flat → colorchannelmixer, fades → fade filters with alpha,
    // keyframes → alphaextract + geq + alphamerge.
    match decompose_opacity(&spec.opacity_curve, dur) {
        Some((base, fi, fo_st, fo_d)) => {
            if let Some(d) = fi {
                if d > 1e-3 {
                    chain.push_str(&format!(",fade=t=in:st=0:d={:.3}:alpha=1", d));
                }
            }
            if let (Some(st), Some(d)) = (fo_st, fo_d) {
                if d > 1e-3 {
                    chain.push_str(&format!(",fade=t=out:st={:.3}:d={:.3}:alpha=1", st, d));
                }
            }
            if (base - 1.0).abs() > 2e-3 {
                chain.push_str(&format!(",colorchannelmixer=aa={:.3}", base.clamp(0.0, 1.0)));
            }
        }
        None => {
            let expr = curve_expression(&spec.opacity_curve, "T");
            let k = index;
            g.parts.push(format!("{chain},format=yuva444p,split=2[ka{k}][kb{k}]", chain = chain, k = k));
            g.parts.push(format!("[kb{k}]alphaextract,geq=lum='{expr}',format=gray[kc{k}]", k = k, expr = expr));
            chain = format!("[ka{k}][kc{k}]alphamerge,format=rgba", k = k);
        }
    }

    // Grade (exposure / eq / colorbalance / curves / colortemperature / lut3d).
    if !spec.grade_filters.is_empty() {
        chain.push_str(&format!(",{}", spec.grade_filters));
    }
    chain.push_str(&format!(",setpts=PTS-STARTPTS+{:.3}/TB[{label}]", spec.start, label = label));
    g.parts.push(chain);

    // ---- placement + blending ------------------------------------------
    let w = spec.w;
    let h = spec.h;
    let x_expr = if !spec.pos_x_keys.is_empty() {
        let e = transform_expr(&spec.pos_x_keys, spec.transform.pos_x, "t");
        format!("'(W-w)/2+({e})*W'")
    } else if spec.transform.pos_x.abs() > 1e-6 {
        format!("'(W-w)/2+{:.6}*W'", spec.transform.pos_x)
    } else {
        "(W-w)/2".to_string()
    };
    let y_expr = if !spec.pos_y_keys.is_empty() {
        let e = transform_expr(&spec.pos_y_keys, spec.transform.pos_y, "t");
        format!("'(H-h)/2+({e})*H'")
    } else if spec.transform.pos_y.abs() > 1e-6 {
        format!("'(H-h)/2+{:.6}*H'", spec.transform.pos_y)
    } else {
        "(H-h)/2".to_string()
    };

    let placed = format!("p{}", g.parts.len());
    match spec.blend.ffmpeg_mode() {
        None => {
            let out = format!("o{}", g.parts.len());
            g.parts.push(format!(
                "[{cur}][{label}]overlay=x={x}:y={y}:eof_action=pass[{out}]",
                cur = g.current,
                label = label,
                x = x_expr,
                y = y_expr,
                out = out,
            ));
            g.current = out;
        }
        Some(mode) => {
            // Position the layer on a transparent canvas, then RGB-blend the
            // whole canvas onto the composite. blend works on the 3 color
            // planes (gbrp); the composite stays fully opaque.
            let canvas = format!("cv{}", g.parts.len());
            g.parts.push(format!(
                "color=c=black@0:s={w}x{h}:r={fps}:d={dur}[{canvas}]",
                w = w,
                h = h,
                fps = spec.fps_s,
                dur = secs(spec.duration),
                canvas = canvas,
            ));
            g.parts.push(format!(
                "[{canvas}][{label}]overlay=x={x}:y={y}:eof_action=pass[{placed}]",
                canvas = canvas,
                label = label,
                x = x_expr,
                y = y_expr,
                placed = placed,
            ));
            let c_rgb = format!("cb{}", g.parts.len());
            let p_rgb = format!("pb{}", g.parts.len());
            g.parts.push(format!("[{cur}]format=gbrp[{c_rgb}]", cur = g.current, c_rgb = c_rgb));
            g.parts.push(format!("[{placed}]format=gbrp[{p_rgb}]", placed = placed, p_rgb = p_rgb));
            let out = format!("o{}", g.parts.len());
            g.parts.push(format!(
                "[{c_rgb}][{p_rgb}]blend=all_mode={mode},format=rgba[{out}]",
                c_rgb = c_rgb,
                p_rgb = p_rgb,
                mode = mode,
                out = out,
            ));
            g.current = out;
        }
    }
}

struct AudioLayerSpec {
    asset_path: std::path::PathBuf,
    in_point: f64,
    source_dur: f64,
    start: f64,
    speed: f64,
    /// Clip-local volume curve (keyframes + fades folded in).
    volume_curve: crate::project::Curve,
    /// Track fader (static).
    track_volume: f64,
    /// Track pan (balance style, -1..1).
    pan: f64,
    /// Track parametric EQ.
    eq: crate::dsp::EqState,
    /// Track multiband compressor.
    comp: crate::dsp::CompState,
    sample_rate: u32,
}

fn add_audio_layer(g: &mut GraphCtx, args: &mut Vec<String>, spec: &AudioLayerSpec) {
    let index = push_input(args, &spec.asset_path, &[
        "-ss", &secs(spec.in_point), "-t", &secs(spec.source_dur),
    ]);
    let delay_ms = (spec.start * 1000.0).round().max(0.0) as i64;
    let k = index;

    // Phase 1: input → resample/tempo/EQ, ending at a named label.
    let mut chain = format!(
        "[{index}:a]aresample={sr},aformat=sample_fmts=fltp:channel_layouts=stereo{at}",
        index = index,
        sr = spec.sample_rate,
        at = atempo_chain(spec.speed),
    );

    // Track EQ — 6-band parametric mapped onto native filters.
    if spec.eq.enabled {
        for b in spec.eq.bands.iter().filter(|b| b.enabled) {
            match b.kind {
                crate::dsp::BandKind::LowCut => {
                    chain.push_str(&format!(",highpass=f={:.0}:poles=2", b.freq.clamp(10.0, 20_000.0)));
                }
                crate::dsp::BandKind::HighCut => {
                    chain.push_str(&format!(",lowpass=f={:.0}:poles=2", b.freq.clamp(100.0, 20_000.0)));
                }
                crate::dsp::BandKind::LowShelf => {
                    chain.push_str(&format!(
                        ",bass=f={:.0}:g={:.2}:width_type=q:width={:.2}",
                        b.freq.clamp(20.0, 2_000.0),
                        b.gain_db.clamp(-18.0, 18.0),
                        b.q.max(0.1),
                    ));
                }
                crate::dsp::BandKind::HighShelf => {
                    chain.push_str(&format!(
                        ",treble=f={:.0}:g={:.2}:width_type=q:width={:.2}",
                        b.freq.clamp(1_000.0, 20_000.0),
                        b.gain_db.clamp(-18.0, 18.0),
                        b.q.max(0.1),
                    ));
                }
                crate::dsp::BandKind::Peaking => {
                    chain.push_str(&format!(
                        ",equalizer=f={:.0}:g={:.2}:width_type=q:width={:.2}",
                        b.freq.clamp(20.0, 20_000.0),
                        b.gain_db.clamp(-18.0, 18.0),
                        b.q.max(0.1),
                    ));
                }
            }
        }
    }

    // Phase 2: optional multiband compressor — split → compress → remix.
    // `bus` is the label the rest of the chain (pan/volume/delay) continues
    // from.
    let bus = if spec.comp.enabled {
        let x1 = spec.comp.xover_low_hz.clamp(40.0, 2_000.0);
        let x2 = spec.comp.xover_high_hz.clamp(1_000.0, 12_000.0);
        chain.push_str(&format!(",asplit=3[c{k}a][c{k}b][c{k}c]"));
        g.parts.push(std::mem::take(&mut chain));
        let comp_args = |b: &crate::dsp::CompBand| {
            if b.enabled {
                format!(
                    "acompressor=threshold={:.1}dB:ratio={:.2}:attack={:.1}:release={:.1}:makeup={:.2}",
                    b.threshold_db.clamp(-60.0, 0.0),
                    b.ratio.clamp(1.0, 20.0),
                    b.attack_ms.clamp(0.1, 500.0),
                    b.release_ms.clamp(5.0, 2_000.0),
                    10f64.powf(b.gain_db.clamp(-24.0, 24.0) / 20.0),
                )
            } else {
                "anull".to_string()
            }
        };
        g.parts.push(format!("[c{k}a]lowpass=f={x1:.0},{}[c{k}lo]", comp_args(&spec.comp.low), k = k, x1 = x1));
        g.parts.push(format!(
            "[c{k}b]highpass=f={x1:.0},lowpass=f={x2:.0},{}[c{k}mid]",
            comp_args(&spec.comp.mid),
            k = k,
            x1 = x1,
            x2 = x2,
        ));
        g.parts.push(format!("[c{k}c]highpass=f={x2:.0},{}[c{k}hi]", comp_args(&spec.comp.high), k = k, x2 = x2));
        g.parts.push(format!("[c{k}lo][c{k}mid][c{k}hi]amix=inputs=3:normalize=0[bus{k}]"));
        format!("[bus{k}]")
    } else {
        chain.push_str(&format!("[pre{k}]"));
        g.parts.push(std::mem::take(&mut chain));
        format!("[pre{k}]")
    };

    // Phase 3: pan → volume → delay, continuing from `bus`.
    let mut chain = bus;
    let mut first = true;

    // Pan (balance style: center = unity). Note: pan gain terms must be
    // written gain-first (`0.75*c0`), not `c0*0.75`.
    let pan = spec.pan.clamp(-1.0, 1.0);
    if pan.abs() > 1e-3 {
        let gl = 1.0 - pan.max(0.0);
        let gr = 1.0 + pan.min(0.0);
        chain.push_str(&format!("pan=stereo|c0={:.4}*c0|c1={:.4}*c1", gl, gr));
        first = false;
    }

    // Volume: static (clip × track), fades, or keyframed expression.
    let clip_curve = &spec.volume_curve;
    let static_vol = spec.track_volume.clamp(0.0, 4.0);
    let flat = clip_curve.first().map(|p| p.1).unwrap_or(1.0);
    let is_flat = clip_curve.iter().all(|p| (p.1 - flat).abs() < 2e-3);
    let sep = if first { "" } else { "," };
    if is_flat {
        let total = (static_vol * flat.clamp(0.0, 4.0)).clamp(0.0, 8.0);
        chain.push_str(&format!("{sep}volume={:.4}", total));
    } else {
        if (static_vol - 1.0).abs() > 1e-3 {
            chain.push_str(&format!("{sep}volume={:.4}", static_vol));
            first = false;
        }
        let sep = if first { "" } else { "," };
        match decompose_volume(clip_curve, spec.source_dur / spec.speed.max(1e-6)) {
            Some((base, fi, fo_st, fo_d)) => {
                if (base - 1.0).abs() > 2e-3 {
                    chain.push_str(&format!("{sep}volume={:.4}", base.clamp(0.0, 4.0)));
                    first = false;
                }
                let sep = if first { "" } else { "," };
                if let Some(d) = fi {
                    if d > 1e-3 {
                        chain.push_str(&format!("{sep}afade=t=in:st=0:d={:.3}:curve=tri", d));
                        first = false;
                    }
                }
                let sep = if first { "" } else { "," };
                if let (Some(st), Some(d)) = (fo_st, fo_d) {
                    if d > 1e-3 {
                        chain.push_str(&format!("{sep}afade=t=out:st={:.3}:d={:.3}:curve=tri", st, d));
                    }
                }
            }
            None => {
                let expr = curve_expression(clip_curve, "t");
                let sep = if first { "" } else { "," };
                chain.push_str(&format!("{sep}volume=volume='{expr}':eval=frame"));
            }
        }
    }

    chain.push_str(&format!(",adelay={dms}:all=1[au{index}]", dms = delay_ms, index = index));
    g.parts.push(chain);
    g.audio_labels.push(format!("[au{index}]"));
}

/// Decompose a volume curve into (base, fade-in, fade-out-start, fade-out-len)
/// or None when keyframed. Mirrors `decompose_opacity` but for raw volume.
fn decompose_volume(curve: &crate::project::Curve, dur: f64) -> Option<(f64, Option<f64>, Option<f64>, Option<f64>)> {
    if curve.is_empty() {
        return Some((1.0, None, None, None));
    }
    let v0 = curve[0].1;
    if curve.iter().all(|p| (p.1 - v0).abs() < 2e-3) {
        return Some((v0, None, None, None));
    }
    let base = curve.iter().cloned().fold(0.0f64, |a, p| a.max(p.1));
    if base <= 1e-3 {
        return None;
    }
    let mut fade_in: Option<f64> = None;
    let mut fade_out: Option<(f64, f64)> = None;
    for w in curve.windows(2) {
        let (t0, a) = w[0];
        let (t1, b) = w[1];
        let n0 = a / base;
        let n1 = b / base;
        let ramp_up = n0.abs() < 2e-3 && (n1 - 1.0).abs() < 2e-3;
        let ramp_dn = (n0 - 1.0).abs() < 2e-3 && n1.abs() < 2e-3;
        if ramp_up {
            if t0.abs() > 1e-3 || fade_in.is_some() {
                return None;
            }
            fade_in = Some(t1 - t0);
        } else if ramp_dn {
            if fade_out.is_some() || (t1 - dur).abs() > 2e-2 {
                return None;
            }
            fade_out = Some((t0, t1 - t0));
        } else if (n0 - n1).abs() > 2e-3 {
            return None;
        }
    }
    Some((base, fade_in, fade_out.map(|f| f.0), fade_out.map(|f| f.1)))
}

pub struct RenderPlan {
    pub cmd: Command,
    pub total_duration: f64,
    pub output: PathBuf,
}

/// Resolve the effective output resolution.
pub fn resolve_size(project: &Project, res: ResChoice) -> (u32, u32) {
    let even = |v: u32| v.max(16) / 2 * 2;
    match res {
        ResChoice::Project => (even(project.width), even(project.height)),
        ResChoice::Fixed(w, h) => (even(w), even(h)),
        ResChoice::FitWidth(w) => {
            let w = even(w);
            let aspect = project.height as f64 / project.width as f64;
            (w, even((w as f64 * aspect).round() as u32))
        }
    }
}

pub fn resolve_fps(project: &Project, fps: FpsChoice) -> f64 {
    match fps {
        FpsChoice::Project => project.fps,
        FpsChoice::Fixed(v) => v,
    }
}

/// Build the complete ffmpeg command for exporting `project`.
pub fn build_export_command(
    locator: &FfmpegLocator,
    project: &Project,
    settings: &ExportSettings,
) -> Result<RenderPlan> {
    let total = project.duration();
    if total <= 0.0 {
        return Err(VeditError::other("Timeline is empty — nothing to export."));
    }

    let (w, h) = resolve_size(project, settings.res);
    let fps = resolve_fps(project, settings.fps);
    let fps_s = format!("{:.4}", fps);

    let video_wanted = settings.video.enabled && is_video_container(settings.container);
    let audio_wanted = settings.audio.enabled && total > 0.0;
    let audio_container_only = is_audio_container(settings.container);
    let is_gif = settings.container == "gif";

    if audio_container_only && !video_wanted && !audio_wanted {
        return Err(VeditError::other("Audio export disabled — nothing to export."));
    }

    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
        "-nostats".into(),
        "-progress".into(),
        "pipe:1".into(),
    ];
    let mut g = GraphCtx {
        parts: Vec::new(),
        audio_labels: Vec::new(),
        current: "bg".to_string(),
    };

    // Background + video layers (bottom → top), unless audio-only export.
    if video_wanted {
        g.parts.push(format!(
            "color=c=black:s={w}x{h}:r={fps}:d={total},format=rgba[bg]",
            w = w,
            h = h,
            fps = fps_s,
            total = secs(total)
        ));
    }

    // Solo semantics: when any track is soloed, only solo tracks are audible.
    let any_solo = project.tracks.iter().any(|t| t.solo);
    let track_audible = |t: &crate::project::Track| !t.muted && (!any_solo || t.solo);

    // Video tracks: media layers, text layers, transitions + their audio.
    for track in project.tracks.iter().filter(|t| t.kind == TrackKind::Video) {
        for (idx, clip) in track.clips.iter().enumerate() {
            if clip.is_text() {
                // Text/title layer.
                if video_wanted {
                    let text = clip.text.clone().unwrap_or_default();
                    let png = crate::textrender::render_text_png_cached(
                        &text,
                        w,
                        h,
                        (text.size as f32 * w as f32 / project.width.max(1) as f32).max(6.0),
                    )
                    .map_err(|e| VeditError::Other(format!("text render failed: {e}")))?;
                    let extra_fade_out = timeline::effective_overlap(project, track.id, clip);
                    add_video_layer(
                        &mut g,
                        &mut args,
                        &VideoLayerSpec {
                            asset_path: &png,
                            text_png: Some(&png),
                            is_image: true,
                            in_point: 0.0,
                            source_dur: 0.0,
                            start: clip.start,
                            duration: clip.duration,
                            opacity_curve: clip.opacity_curve(extra_fade_out),
                            grade_filters: grade_filters_lenient(clip.grade.as_ref()),
                            w,
                            h,
                            fps_s: &fps_s,
                            crop: Default::default(),
                            blend: clip.blend,
                            transform: Default::default(),
                            scale_keys: Vec::new(),
                            pos_x_keys: Vec::new(),
                            pos_y_keys: Vec::new(),
                            rot_keys: Vec::new(),
                        },
                    );
                }
                continue;
            }

            let Some(asset) = project.asset(clip.asset_id) else { continue };
            let is_image = asset.kind == MediaKind::Image;

            // Cross-dissolve pull-in: when the PREVIOUS clip on this track
            // transitions into this one, the layer starts early and fades in.
            let pull = if idx > 0 {
                timeline::effective_overlap(project, track.id, &track.clips[idx - 1])
            } else {
                0.0
            };

            if video_wanted && (asset.kind == MediaKind::Video || is_image) {
                let (start, in_point, source_dur, duration) = if pull > 0.0 {
                    (
                        clip.start - pull,
                        (clip.in_point - pull * clip.speed).max(0.0),
                        clip.source_duration() + pull * clip.speed,
                        clip.duration + pull,
                    )
                } else {
                    (clip.start, clip.in_point, clip.source_duration(), clip.duration)
                };
                // Outgoing fade from the previous clip's transition.
                let own_extra = timeline::effective_overlap(project, track.id, clip);
                let curve = if pull > 0.0 {
                    // Follower: fade in over [0, pull], then its own curve.
                    let mut c: crate::project::Curve = vec![(0.0, 0.0)];
                    for (t, v) in clip.opacity_curve(own_extra) {
                        c.push((t + pull, v));
                    }
                    c
                } else {
                    clip.opacity_curve(own_extra)
                };
                add_video_layer(
                    &mut g,
                    &mut args,
                    &VideoLayerSpec {
                        asset_path: &asset.path,
                        text_png: None,
                        is_image,
                        in_point,
                        source_dur,
                        start,
                        duration,
                        opacity_curve: curve,
                        grade_filters: grade_filters_lenient(clip.grade.as_ref()),
                        w,
                        h,
                        fps_s: &fps_s,
                        crop: clip.crop,
                        blend: clip.blend,
                        transform: clip.transform,
                        scale_keys: clip
                            .keyframes
                            .iter()
                            .filter_map(|kf| kf.scale_x.map(|v| (kf.t, v)))
                            .collect(),
                        pos_x_keys: clip
                            .keyframes
                            .iter()
                            .filter_map(|kf| kf.pos_x.map(|v| (kf.t, v)))
                            .collect(),
                        pos_y_keys: clip
                            .keyframes
                            .iter()
                            .filter_map(|kf| kf.pos_y.map(|v| (kf.t, v)))
                            .collect(),
                        rot_keys: clip
                            .keyframes
                            .iter()
                            .filter_map(|kf| kf.rotation.map(|v| (kf.t, v)))
                            .collect(),
                    },
                );
            }

            // Audio carried by a video file on a video track. Linked clips
            // are skipped: their audio comes from the dedicated audio-track
            // twin (mixing both would double the sound).
            let linked = clip.link.is_some();
            if audio_wanted && asset.has_audio() && !linked && track_audible(track) && !clip.muted {
                add_audio_layer(
                    &mut g,
                    &mut args,
                    &AudioLayerSpec {
                        asset_path: asset.path.clone(),
                        in_point: clip.in_point,
                        source_dur: clip.source_duration(),
                        start: clip.start,
                        speed: clip.speed,
                        volume_curve: clip.volume_curve(),
                        track_volume: track.volume,
                        pan: track.pan,
                        eq: track.eq.clone(),
                        comp: track.comp.clone(),
                        sample_rate: settings.audio.sample_rate,
                    },
                );
            }
        }
    }

    // Dedicated audio-track clips.
    for track in project.tracks.iter().filter(|t| t.kind == TrackKind::Audio) {
        for clip in &track.clips {
            if clip.is_text() {
                continue;
            }
            let Some(asset) = project.asset(clip.asset_id) else { continue };
            if !audio_wanted || !asset.has_audio() || !track_audible(track) || clip.muted {
                continue;
            }
            add_audio_layer(
                &mut g,
                &mut args,
                &AudioLayerSpec {
                    asset_path: asset.path.clone(),
                    in_point: clip.in_point,
                    source_dur: clip.source_duration(),
                    start: clip.start,
                    speed: clip.speed,
                    volume_curve: clip.volume_curve(),
                    track_volume: track.volume,
                    pan: track.pan,
                    eq: track.eq.clone(),
                    comp: track.comp.clone(),
                    sample_rate: settings.audio.sample_rate,
                },
            );
        }
    }

    if args.is_empty() {
        return Err(VeditError::other("No media clips found on the timeline."));
    }

    // Close the video graph.
    if video_wanted {
        if is_gif {
            g.parts.push(format!(
                "[{cur}]split[gs1][gs2];[gs1]palettegen=stats_mode=diff[gp];[gs2][gp]paletteuse=dither=bayer:bayer_scale=4[vgif]",
                cur = g.current
            ));
        } else {
            let pix = video_codec_def(settings.video.codec)
                .and_then(|d| d.pix_fmt)
                .unwrap_or("yuv420p");
            g.parts.push(format!("[{cur}]format={pix}[vout]", cur = g.current, pix = pix));
        }
    }

    // Close the audio graph.
    let has_audio_graph = audio_wanted && !g.audio_labels.is_empty();
    if has_audio_graph {
        let joined = g.audio_labels.join("");
        let master = project.master_volume.clamp(0.0, 2.0);
        if (master - 1.0).abs() > 1e-3 {
            g.parts.push(format!(
                "{joined}amix=inputs={n}:normalize=0:dropout_transition=0,volume={master:.4}[aout]",
                n = g.audio_labels.len(),
                master = master
            ));
        } else {
            g.parts.push(format!(
                "{joined}amix=inputs={n}:normalize=0:dropout_transition=0[aout]",
                n = g.audio_labels.len()
            ));
        }
    }

    if g.parts.is_empty() {
        return Err(VeditError::other("Nothing mapped for export."));
    }

    args.push("-filter_complex".into());
    args.push(g.parts.join(";"));

    if video_wanted {
        if is_gif {
            args.push("-map".into());
            args.push("[vgif]".into());
        } else {
            args.push("-map".into());
            args.push("[vout]".into());
        }
    }
    if has_audio_graph {
        args.push("-map".into());
        args.push("[aout]".into());
    }

    // Encoder settings — video.
    if video_wanted && !is_gif {
        let def = video_codec_def(settings.video.codec)
            .ok_or_else(|| VeditError::other("unknown video codec"))?;
        args.push("-c:v".into());
        args.push(def.id.to_string());
        if !def.lossless {
            match settings.video.rate {
                RateMode::Bitrate { kbps } => {
                    args.push("-b:v".into());
                    args.push(format!("{}k", kbps.max(16)));
                }
                RateMode::Crf { crf } => {
                    if def.supports_crf {
                        args.push("-crf".into());
                        args.push(format!("{}", crf.clamp(0, 51)));
                        for (k, v) in def.crf_extra {
                            args.push(k.to_string());
                            args.push(v.to_string());
                        }
                    } else {
                        args.push("-b:v".into());
                        args.push(format!("{}k", def.default_bitrate_kbps.max(16)));
                    }
                }
            }
        }
        if let Some(preset) = settings.video.preset {
            if def.presets.contains(&preset) {
                args.push("-preset".into());
                args.push(preset.to_string());
            }
        }
        for (k, v) in def.extra {
            args.push(k.to_string());
            args.push(v.to_string());
        }
        if let Some(pix) = def.pix_fmt {
            args.push("-pix_fmt".into());
            args.push(pix.to_string());
        }
        if matches!(settings.container, "mp4" | "mov" | "m4v") {
            args.push("-movflags".into());
            args.push("+faststart".into());
        }
    }

    // Encoder settings — audio.
    if has_audio_graph {
        let adef = audio_codec_def(settings.audio.codec).unwrap_or(&AUDIO_CODECS[0]);
        args.push("-c:a".into());
        args.push(adef.id.to_string());
        if !adef.lossless {
            args.push("-b:a".into());
            args.push(format!("{}k", settings.audio.bitrate_kbps.max(32)));
        }
        if adef.id != "libopus" || settings.audio.sample_rate >= 16_000 {
            args.push("-ar".into());
            args.push(format!("{}", settings.audio.sample_rate));
        }
    }

    // User extras.
    for extra in settings.extra_args.split_whitespace() {
        args.push(extra.to_string());
    }

    // Output path (container from extension).
    let mut out_path = settings.output.clone();
    let wanted_ext = settings.container;
    if out_path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()) != Some(wanted_ext.into()) {
        out_path.set_extension(wanted_ext);
    }
    ensure_parent_dir(&out_path)?;
    args.push(out_path.display().to_string());

    let mut cmd = base_command(locator.ffmpeg());
    cmd.args(&args).stdin(Stdio::null());
    log::debug!("export command: ffmpeg {}", args.join(" "));

    Ok(RenderPlan {
        cmd,
        total_duration: total,
        output: out_path,
    })
}

// ---------------------------------------------------------------------------
// Export job (background render with progress + cancel)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ExportEvt {
    Progress { pct: f32, out_time: f64 },
    Done { output: PathBuf },
    Failed { message: String },
}

pub struct ExportJob {
    rx: Receiver<ExportEvt>,
    kill: Arc<Mutex<Option<Child>>>,
    pub started: Instant,
    pub output: PathBuf,
    pub total_duration: f64,
}

impl ExportJob {
    pub fn start(mut plan: RenderPlan) -> Result<Self> {
        let output = plan.output.clone();
        let total = plan.total_duration;
        let mut child = plan
            .cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| VeditError::Other(format!("failed to launch ffmpeg: {e}")))?;
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        let (tx, rx) = std::sync::mpsc::channel::<ExportEvt>();
        let kill: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(Some(child)));
        let err_tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));

        // Drain stderr so ffmpeg never blocks on a full pipe.
        {
            let err_tail = err_tail.clone();
            std::thread::spawn(move || {
                use std::io::BufRead;
                let reader = std::io::BufReader::new(stderr);
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    let mut tail = err_tail.lock().unwrap();
                    tail.push_back(line);
                    while tail.len() > 40 {
                        tail.pop_front();
                    }
                }
            });
        }

        // Progress reader + finisher.
        {
            let kill = kill.clone();
            let err_tail = err_tail.clone();
            let output = output.clone();
            std::thread::spawn(move || {
                use std::io::BufRead;
                let reader = std::io::BufReader::new(stdout);
                let mut render_result: Result<()> = Ok(());
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    let line = line.trim().to_string();
                    if let Some(v) = line.strip_prefix("out_time_us=") {
                        if let Ok(us) = v.parse::<f64>() {
                            let out_time = us / 1_000_000.0;
                            let pct = ((out_time / total) * 100.0).clamp(0.0, 99.5) as f32;
                            let _ = tx.send(ExportEvt::Progress { pct, out_time });
                        }
                    } else if line == "progress=end" {
                        break;
                    }
                }
                // Wait for exit.
                let status = {
                    let mut guard = kill.lock().unwrap();
                    guard.as_mut().map(|c| c.wait())
                };
                if let Some(Ok(st)) = status {
                    if !st.success() {
                        let msg = err_tail
                            .lock()
                            .unwrap()
                            .iter()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("\n");
                        render_result = Err(VeditError::ToolFailed(if msg.is_empty() {
                            format!("ffmpeg exited with {st}")
                        } else {
                            msg
                        }));
                    }
                }
                *kill.lock().unwrap() = None;
                match render_result {
                    Ok(()) => {
                        let _ = tx.send(ExportEvt::Done { output: output.clone() });
                    }
                    Err(e) => {
                        let _ = tx.send(ExportEvt::Failed { message: e.to_string() });
                    }
                }
            });
        }

        Ok(ExportJob {
            rx,
            kill,
            started: Instant::now(),
            output,
            total_duration: total,
        })
    }

    /// Poll without blocking; call every UI frame.
    pub fn poll(&mut self) -> Option<ExportEvt> {
        self.rx.try_recv().ok()
    }

    /// Cancel the running render.
    pub fn cancel(&self) {
        if let Some(c) = self.kill.lock().unwrap().as_mut() {
            kill_child(c);
        }
    }

    pub fn running(&self) -> bool {
        self.kill.lock().unwrap().is_some()
    }
}

impl Drop for ExportJob {
    fn drop(&mut self) {
        self.cancel();
    }
}
