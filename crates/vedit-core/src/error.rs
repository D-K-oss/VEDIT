use std::path::PathBuf;

/// Unified error type for the VEDIT engine.
#[derive(Debug, thiserror::Error)]
pub enum VeditError {
    #[error(
        "ffmpeg/ffprobe not found ({hint}). Bundle ffmpeg next to vedit.exe (folder `ffmpeg/` beside the executable) or install ffmpeg and add it to PATH."
    )]
    FfmpegMissing { hint: String },

    #[error("external tool failed:\n{0}")]
    ToolFailed(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("image error: {0}")]
    Image(#[from] image::ImageError),

    #[error("{0}")]
    Other(String),
}

impl VeditError {
    pub fn other<S: Into<String>>(msg: S) -> Self {
        VeditError::Other(msg.into())
    }
}

pub type Result<T> = std::result::Result<T, VeditError>;

/// Convenience: human readable file size.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut unit = 0usize;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

/// Ensure the parent directory of a path exists (used before exports/saves).
pub fn ensure_parent_dir(path: &PathBuf) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}
