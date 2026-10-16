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
        extra: &[],
        crf_extra: &[],
        default_bitrate_kbps: 0,
    },
];

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

#[allow(clippy::too_many_arguments)]
fn add_video_layer(
    g: &mut GraphCtx,
    args: &mut Vec<String>,
    asset_path: &std::path::Path,
    is_image: bool,
    in_point: f64,
    source_dur: f64,
    start: f64,
    duration: f64,
    opacity: f64,
    w: u32,
    h: u32,
    fps_s: &str,
    cur: &mut String,
) {
    let index = if is_image {
        push_input(args, asset_path, &["-loop", "1", "-t", &secs(duration)])
    } else {
        push_input(args, asset_path, &["-ss", &secs(in_point), "-t", &secs(source_dur)])
    };

    let chain = format!(
        "[{i}:v]fps={fps},scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,format=rgba,colorchannelmixer=aa={op:.3},setpts=PTS-STARTPTS+{start:.3}/TB[v{i}]",
        i = index,
        fps = fps_s,
        w = w,
        h = h,
        op = opacity.clamp(0.0, 1.0),
        start = start,
    );
    g.parts.push(chain);

    let out = format!("o{}", g.parts.len());
    g.parts.push(format!("[{cur}][v{index}]overlay=0:0:eof_action=pass[{out}]"));
    *cur = out;
}

#[allow(clippy::too_many_arguments)]
fn add_audio_layer(
    g: &mut GraphCtx,
    args: &mut Vec<String>,
    asset_path: &std::path::Path,
    in_point: f64,
    source_dur: f64,
    start: f64,
    speed: f64,
    volume: f64,
    sample_rate: u32,
) {
    let index = push_input(args, asset_path, &["-ss", &secs(in_point), "-t", &secs(source_dur)]);
    let delay_ms = (start * 1000.0).round().max(0.0) as i64;
    let chain = format!(
        "[{i}:a]aresample={sr},aformat=sample_fmts=fltp:channel_layouts=stereo{at},volume={vol:.4},adelay={dms}:all=1[au{i}]",
        i = index,
        sr = sample_rate,
        at = atempo_chain(speed),
        vol = volume.clamp(0.0, 4.0),
        dms = delay_ms,
    );
    g.parts.push(chain);
    g.audio_labels.push(format!("[au{index}]"));
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
    let mut g = GraphCtx { parts: Vec::new(), audio_labels: Vec::new() };
    let mut cur = "bg".to_string();

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

    for track in project.tracks.iter().filter(|t| t.kind == TrackKind::Video) {
        for clip in &track.clips {
            let Some(asset) = project.asset(clip.asset_id) else { continue };
            let is_image = asset.kind == MediaKind::Image;
            if video_wanted && (asset.kind == MediaKind::Video || is_image) {
                let source_dur = clip.source_duration();
                add_video_layer(
                    &mut g,
                    &mut args,
                    &asset.path,
                    is_image,
                    clip.in_point,
                    source_dur,
                    clip.start,
                    clip.duration,
                    clip.opacity,
                    w,
                    h,
                    &fps_s,
                    &mut cur,
                );
            }
            // Audio carried by a video file on a video track.
            if audio_wanted && asset.has_audio() && !track.muted && !clip.muted {
                add_audio_layer(
                    &mut g,
                    &mut args,
                    &asset.path,
                    clip.in_point,
                    clip.source_duration(),
                    clip.start,
                    clip.speed,
                    clip.volume,
                    settings.audio.sample_rate,
                );
            }
        }
    }

    // Dedicated audio-track clips.
    for track in project.tracks.iter().filter(|t| t.kind == TrackKind::Audio) {
        for clip in &track.clips {
            let Some(asset) = project.asset(clip.asset_id) else { continue };
            if !audio_wanted || !asset.has_audio() || track.muted || clip.muted {
                continue;
            }
            add_audio_layer(
                &mut g,
                &mut args,
                &asset.path,
                clip.in_point,
                clip.source_duration(),
                clip.start,
                clip.speed,
                clip.volume,
                settings.audio.sample_rate,
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
                "[{cur}]split[gs1][gs2];[gs1]palettegen=stats_mode=diff[gp];[gs2][gp]paletteuse=dither=bayer:bayer_scale=4[vgif]"
            ));
        } else {
            let pix = video_codec_def(settings.video.codec)
                .and_then(|d| d.pix_fmt)
                .unwrap_or("yuv420p");
            g.parts.push(format!("[{cur}]format={pix}[vout]"));
        }
    }

    // Close the audio graph.
    let has_audio_graph = audio_wanted && !g.audio_labels.is_empty();
    if has_audio_graph {
        let joined = g.audio_labels.join("");
        g.parts.push(format!(
            "{joined}amix=inputs={n}:normalize=0:dropout_transition=0[aout]",
            n = g.audio_labels.len()
        ));
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
