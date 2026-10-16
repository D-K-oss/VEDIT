//! Thumbnails for the media library.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::ffmpeg::{base_command, run_checked, FfmpegLocator};
use crate::media::MediaAsset;

/// Where thumbnails are cached on disk.
pub fn cache_dir() -> PathBuf {
    std::env::temp_dir().join("vedit-cache")
}

fn cache_path(asset_id: u64) -> PathBuf {
    cache_dir().join(format!("thumb-{asset_id}.png"))
}

/// Extract a poster frame for a video asset at `at_time` seconds.
pub fn extract_video_thumbnail(ffmpeg: &FfmpegLocator, path: &Path, at_time: f64, out_png: &Path) -> Result<()> {
    let mut cmd = base_command(ffmpeg.ffmpeg());
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-ss",
        &format!("{at_time:.3}"),
        "-i",
    ])
    .arg(path)
    .args(["-frames:v", "1", "-vf", "scale=320:-2", "-y"])
    .arg(out_png);
    run_checked(&mut cmd)
}

/// Decode + downscale an image asset to a thumbnail.
pub fn image_thumbnail(path: &Path) -> Option<image::RgbaImage> {
    let img = image::open(path).ok()?;
    Some(downscale_rgba(&img, 320))
}

/// Load a cached thumbnail file as RGBA.
pub fn load_cached(asset_id: u64) -> Option<image::RgbaImage> {
    let p = cache_path(asset_id);
    if p.is_file() {
        image::open(&p).ok().map(|img| img.to_rgba8())
    } else {
        None
    }
}

pub fn store_cache(asset_id: u64, img: &image::RgbaImage) -> Option<PathBuf> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let p = cache_path(asset_id);
    img.save(&p).ok()?;
    Some(p)
}

pub fn downscale_rgba(img: &image::DynamicImage, max_w: u32) -> image::RgbaImage {
    let scale = (max_w as f32 / img.width().max(1) as f32).min(1.0);
    let w = ((img.width() as f32 * scale).round() as u32).max(1);
    let h = ((img.height() as f32 * scale).round() as u32).max(1);
    image::DynamicImage::ImageRgba8(img.to_rgba8())
        .resize_exact(w, h, image::imageops::FilterType::Triangle)
        .to_rgba8()
}

/// Deterministic placeholder for audio assets (fake waveform bars).
pub fn audio_placeholder() -> image::RgbaImage {
    const W: u32 = 320;
    const H: u32 = 90;
    let mut img = image::RgbaImage::from_pixel(W, H, image::Rgba([21, 21, 24, 255]));
    let bar_color = image::Rgba([127, 176, 138, 255]);
    let center = H / 2;
    for x in 0..W {
        let t = x as f32;
        let amp = (0.35 + 0.65 * ((t * 0.09).sin() * (t * 0.023 + 1.7).cos()).abs())
            * (1.0 - (t / W as f32 - 0.5).abs() * 0.6);
        let half = (amp * (H as f32 * 0.42)).round() as u32;
        for y in center.saturating_sub(half)..(center + half).min(H - 1) {
            img.put_pixel(x, y, bar_color);
        }
    }
    img
}

/// Generate (and cache) the library thumbnail for an asset.
/// Runs on background threads.
pub fn generate_thumbnail(ffmpeg: &FfmpegLocator, asset: &MediaAsset) -> Option<image::RgbaImage> {
    if let Some(cached) = load_cached(asset.id) {
        return Some(cached);
    }
    let img = match asset.kind {
        crate::media::MediaKind::Video => {
            let at = (asset.info.duration.unwrap_or(1.0) * 0.25).min(3.0);
            let out = cache_dir().join(format!("thumb-{}.png", asset.id));
            std::fs::create_dir_all(cache_dir()).ok()?;
            extract_video_thumbnail(ffmpeg, &asset.path, at, &out).ok()?;
            image::open(&out).ok().map(|i| downscale_rgba(&i, 320))?
        }
        crate::media::MediaKind::Image => image_thumbnail(&asset.path)?,
        crate::media::MediaKind::Audio => audio_placeholder(),
    };
    store_cache(asset.id, &img);
    Some(img)
}
