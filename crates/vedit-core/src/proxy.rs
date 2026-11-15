//! Optimized media (proxies): 1080p stand-ins for heavy sources.
//!
//! 4K/6K/8K sources decode far slower than realtime on mid-range machines,
//! which is what made 4K timelines freeze. The professional fix is the same
//! one Resolve/Premiere use: generate a lightweight proxy once at import and
//! use it for preview, thumbnails and waveforms. Exports always read the
//! ORIGINAL file, so output quality is untouched.
//!
//! Proxies live in a `.vedit-proxies` folder next to the project (or next to
//! the temp dir when the project is unsaved) and are keyed by a hash of the
//! source path + mtime, so they are reused across sessions and regenerated
//! automatically when a source changes.

use std::path::{Path, PathBuf};

use crate::error::{Result, VeditError};
use crate::ffmpeg::{base_command, run_checked, FfmpegLocator};
use crate::media::{MediaInfo, MediaKind};

/// Long edge at or above which a proxy is generated.
pub const PROXY_THRESHOLD: u32 = 2160; // 4K and above get proxies; 1080p plays natively
/// Proxy long edge.
pub const PROXY_LONG_EDGE: u32 = 1080;

/// True when this asset benefits from a proxy (video above the threshold).
pub fn needs_proxy(kind: MediaKind, info: &MediaInfo) -> bool {
    if kind != MediaKind::Video || !info.has_video {
        return false;
    }
    let w = info.width.unwrap_or(0);
    let h = info.height.unwrap_or(0);
    w.max(h) >= PROXY_THRESHOLD
}

/// Deterministic proxy file name: <hash>.proxy.mp4 next to the project.
fn hash_path(path: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.display().to_string().hash(&mut h);
    if let Ok(md) = std::fs::metadata(path) {
        if let Ok(mtime) = md.modified() {
            if let Ok(d) = mtime.duration_since(std::time::UNIX_EPOCH) {
                d.as_secs().hash(&mut h);
            }
        }
    }
    h.finish()
}

/// Where the proxy for `source` should live (and whether it already exists).
pub fn proxy_path(project_dir: Option<&Path>, source: &Path) -> PathBuf {
    let dir = match project_dir {
        Some(d) => d.join(".vedit-proxies"),
        None => std::env::temp_dir().join("vedit-proxies"),
    };
    dir.join(format!("{:016x}.proxy.mp4", hash_path(source)))
}

/// Generate (or reuse) a 1080p proxy for `source`. Blocking — call from a
/// background thread. Returns the proxy path.
pub fn generate_proxy(locator: &FfmpegLocator, project_dir: Option<&Path>, source: &Path) -> Result<PathBuf> {
    let out = proxy_path(project_dir, source);
    if out.is_file() {
        return Ok(out);
    }
    let dir = out.parent().ok_or_else(|| VeditError::other("proxy path has no parent"))?;
    std::fs::create_dir_all(dir)?;

    // Scale the LONG edge to PROXY_LONG_EDGE preserving aspect (even dims),
    // fast x264 + AAC. Posterize-friendly and ~10x faster to decode than 4K.
    let mut cmd = base_command(locator.ffmpeg());
    cmd.arg("-y").arg("-v").arg("error");
    cmd.arg("-i").arg(source);
    cmd.arg("-vf").arg(format!(
        "scale='min({e},iw)':'min({e},ih)':force_original_aspect_ratio=decrease,scale=trunc(iw/2)*2:trunc(ih/2)*2",
        e = PROXY_LONG_EDGE
    ));
    cmd.arg("-c:v").arg("libx264");
    cmd.arg("-preset").arg("veryfast");
    cmd.arg("-crf").arg("23");
    cmd.arg("-pix_fmt").arg("yuv420p");
    cmd.arg("-c:a").arg("aac");
    cmd.arg("-b:a").arg("192k");
    cmd.arg("-movflags").arg("+faststart");
    cmd.arg(&out);
    if let Err(e) = run_checked(&mut cmd) {
        let _ = std::fs::remove_file(&out);
        return Err(e);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_logic() {
        let info4k = MediaInfo { width: Some(3840), height: Some(2160), has_video: true, has_audio: true, ..Default::default() };
        let info1080 = MediaInfo { width: Some(1920), height: Some(1080), has_video: true, has_audio: true, ..Default::default() };
        assert!(needs_proxy(MediaKind::Video, &info4k));
        assert!(!needs_proxy(MediaKind::Video, &info1080));
        assert!(!needs_proxy(MediaKind::Audio, &info4k));
        assert!(!needs_proxy(MediaKind::Image, &info4k));
    }
}
