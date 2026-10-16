//! ffprobe wrapper: turns any file into structured `MediaInfo`.

use std::path::Path;

use serde::Deserialize;

use crate::error::{Result, VeditError};
use crate::ffmpeg::{base_command, FfmpegLocator};
use crate::media::MediaInfo;

#[derive(Debug, Deserialize)]
struct ProbeRoot {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    #[serde(default)]
    format: ProbeFormat,
}

#[derive(Debug, Deserialize)]
struct ProbeStream {
    #[serde(default)]
    codec_type: Option<String>,
    #[serde(default)]
    codec_name: Option<String>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    r_frame_rate: Option<String>,
    #[serde(default)]
    sample_rate: Option<String>,
    #[serde(default)]
    channels: Option<u32>,
    #[serde(default)]
    duration: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ProbeFormat {
    #[serde(default)]
    duration: Option<String>,
    #[serde(default)]
    format_name: Option<String>,
    #[serde(default)]
    size: Option<String>,
}

fn parse_fraction(s: &str) -> Option<f64> {
    let mut parts = s.splitn(2, '/');
    let num: f64 = parts.next()?.trim().parse().ok()?;
    let den: f64 = match parts.next() {
        Some(d) => d.trim().parse().ok()?,
        None => 1.0,
    };
    if den <= 0.0 || num <= 0.0 {
        return None;
    }
    Some(num / den)
}

fn parse_f64(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Probe a media file with ffprobe (JSON output).
pub fn probe(locator: &FfmpegLocator, path: &Path) -> Result<MediaInfo> {
    let out = base_command(locator.ffprobe())
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .stderr(std::process::Stdio::piped())
        .output()?;
    if !out.status.success() {
        return Err(VeditError::ToolFailed(format!(
            "ffprobe could not read {}\n{}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let root: ProbeRoot = serde_json::from_slice(&out.stdout)?;

    let mut info = MediaInfo::default();
    for st in &root.streams {
        match st.codec_type.as_deref() {
            Some("video") => {
                // Skip cover-art / attached-picture streams (mjpeg in mp3 files).
                let is_cover = st.codec_name.as_deref() == Some("mjpeg")
                    || st.codec_name.as_deref() == Some("png")
                        && root.streams.iter().any(|s| s.codec_type.as_deref() == Some("audio"));
                if is_cover && info.has_video {
                    continue;
                }
                if !info.has_video {
                    info.has_video = true;
                    info.width = st.width;
                    info.height = st.height;
                    info.fps = st.r_frame_rate.as_deref().and_then(parse_fraction);
                    info.codec_video = st.codec_name.clone();
                    info.duration = st.duration.as_deref().and_then(parse_f64);
                }
            }
            Some("audio") => {
                info.has_audio = true;
                if info.audio_sample_rate.is_none() {
                    info.audio_sample_rate = st.sample_rate.as_deref().and_then(parse_f64).map(|v| v as u32);
                    info.audio_channels = st.channels;
                    info.codec_audio = st.codec_name.clone();
                    if info.duration.is_none() {
                        info.duration = st.duration.as_deref().and_then(parse_f64);
                    }
                }
            }
            _ => {}
        }
    }
    if info.duration.is_none() {
        info.duration = root.format.duration.as_deref().and_then(parse_f64);
    }
    info.format_name = root.format.format_name;
    info.size_bytes = root.format.size.as_deref().and_then(parse_f64).map(|v| v as u64);

    // Some images (and some containers) report no duration at all — fine.
    Ok(info)
}
