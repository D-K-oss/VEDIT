//! Media assets: anything the user can import — video, audio or images.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, VeditError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    Video,
    Audio,
    Image,
}

impl MediaKind {
    pub fn label(&self) -> &'static str {
        match self {
            MediaKind::Video => "VIDEO",
            MediaKind::Audio => "AUDIO",
            MediaKind::Image => "IMAGE",
        }
    }
}

/// Technical metadata extracted by ffprobe.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaInfo {
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub has_video: bool,
    pub has_audio: bool,
    pub audio_sample_rate: Option<u32>,
    pub audio_channels: Option<u32>,
    pub codec_video: Option<String>,
    pub codec_audio: Option<String>,
    pub format_name: Option<String>,
    pub size_bytes: Option<u64>,
}

/// An imported media file. Persisted inside the project file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaAsset {
    pub id: u64,
    pub path: PathBuf,
    pub name: String,
    pub kind: MediaKind,
    pub info: MediaInfo,
    /// Optimized-media proxy (1080p stand-in for huge sources). Preview,
    /// thumbnails and waveforms read this; exports read `path`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<PathBuf>,
}

impl MediaAsset {
    /// The file that should be decoded for PREVIEW (proxy when present).
    pub fn preview_path(&self) -> &Path {
        match &self.proxy {
            Some(p) if p.is_file() => p,
            _ => &self.path,
        }
    }

    /// True while a proxy exists on disk.
    pub fn has_proxy(&self) -> bool {
        self.proxy.as_ref().map(|p| p.is_file()).unwrap_or(false)
    }

    /// Duration that matters for timeline purposes. Images have none.
    pub fn duration(&self) -> Option<f64> {
        match self.kind {
            MediaKind::Image => None,
            _ => self.info.duration,
        }
    }

    pub fn has_audio(&self) -> bool {
        self.info.has_audio
    }

    pub fn summary(&self) -> String {
        match self.kind {
            MediaKind::Video => format!(
                "{}×{} · {} · {}",
                self.info.width.unwrap_or(0),
                self.info.height.unwrap_or(0),
                crate::media::format_duration(self.info.duration.unwrap_or(0.0)),
                self.info.codec_video.as_deref().unwrap_or("?")
            ),
            MediaKind::Audio => format!(
                "{}Hz · {}ch · {}",
                self.info.audio_sample_rate.unwrap_or(0),
                self.info.audio_channels.unwrap_or(0),
                crate::media::format_duration(self.info.duration.unwrap_or(0.0))
            ),
            MediaKind::Image => format!(
                "{}×{} · image",
                self.info.width.unwrap_or(0),
                self.info.height.unwrap_or(0)
            ),
        }
    }
}

/// "HH:MM:SS.mmm" formatting used across the UI.
pub fn format_duration(t: f64) -> String {
    let t = t.max(0.0);
    let h = (t / 3600.0).floor() as u32;
    let m = ((t % 3600.0) / 60.0).floor() as u32;
    let s = t % 60.0;
    if h > 0 {
        format!("{h:02}:{m:02}:{s:06.3}")
    } else {
        format!("{m:02}:{s:06.3}")
    }
}

/// Timecode HH:MM:SS:FF at the given (rounded) frame rate.
pub fn format_timecode(t: f64, fps: f64) -> String {
    let fps_display = fps.round().max(1.0) as u32;
    let total_frames = (t * fps as f64).round().max(0.0) as u64;
    let ff = (total_frames % fps_display as u64) as u32;
    let secs_total = (t.max(0.0)).floor() as u32;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        secs_total / 3600,
        (secs_total % 3600) / 60,
        secs_total % 60,
        ff
    )
}

const VIDEO_EXTS: &[&str] = &[
    "mp4", "m4v", "mkv", "mov", "avi", "webm", "flv", "wmv", "mpg", "mpeg", "m2v", "ts", "m2ts",
    "mts", "3gp", "3g2", "ogv", "vob", "asf", "rm", "rmvb", "divx", "f4v", "mxf", "dv",
];
const AUDIO_EXTS: &[&str] = &[
    "mp3", "wav", "aac", "flac", "ogg", "oga", "m4a", "wma", "opus", "aiff", "aif", "alac",
    "amr", "au", "ac3", "dts", "mka", "caf", "mpc", "ape", "wv", "tta",
];
const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "jpe", "gif", "bmp", "webp", "tif", "tiff", "tga", "svg", "avif",
    "heic", "heif", "jp2", "psd", "exr", "hdr", "qoi", "ico", "dds",
];

pub fn extension_of(path: &Path) -> Option<String> {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
}

/// Classify by extension first (fast, predictable), fall back to probing.
pub fn kind_from_extension(path: &Path) -> Option<MediaKind> {
    let ext = extension_of(path)?;
    if VIDEO_EXTS.contains(&ext.as_str()) {
        Some(MediaKind::Video)
    } else if AUDIO_EXTS.contains(&ext.as_str()) {
        Some(MediaKind::Audio)
    } else if IMAGE_EXTS.contains(&ext.as_str()) {
        Some(MediaKind::Image)
    } else {
        None
    }
}

/// Decide the final kind using both the extension and the probe result.
pub fn classify(path: &Path, info: &MediaInfo) -> MediaKind {
    if let Some(kind) = kind_from_extension(path) {
        return kind;
    }
    if info.has_video {
        if info.has_audio || info.duration.unwrap_or(0.0) > 0.05 {
            MediaKind::Video
        } else {
            MediaKind::Image
        }
    } else if info.has_audio {
        MediaKind::Audio
    } else {
        MediaKind::Image
    }
}

/// Build a `MediaAsset` from a path + probe result.
pub fn make_asset(id: u64, path: PathBuf, info: MediaInfo) -> Result<MediaAsset> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or_else(|| VeditError::other("path has no file name"))?;
    let kind = classify(&path, &info);
    Ok(MediaAsset {
        id,
        path,
        name,
        kind,
        info,
        proxy: None,
    })
}

/// Filter lists for the file-Open dialog.
pub const IMPORT_DIALOG_EXTS: &[&str] = &[
    "mp4", "m4v", "mkv", "mov", "avi", "webm", "flv", "wmv", "mpg", "mpeg", "ts", "m2ts", "3gp",
    "ogv", "mp3", "wav", "aac", "flac", "ogg", "m4a", "wma", "opus", "aiff", "ac3", "png", "jpg",
    "jpeg", "gif", "bmp", "webp", "tif", "tiff", "tga", "avif", "heic", "jp2", "exr", "qoi",
];
