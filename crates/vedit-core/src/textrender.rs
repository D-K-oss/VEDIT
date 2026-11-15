//! Text/title rasterization (pure Rust, no system font dependency).
//!
//! Two embedded DejaVu faces (regular + bold, libre-licensed) are rendered
//! with `ab_glyph` into a transparent RGBA image sized to the target frame;
//! the image is composited by the preview and overlaid as a PNG input by the
//! exporter — so preview and export share one text engine.

use ab_glyph::{Font, FontArc, Glyph, PxScaleFont, ScaleFont};
use image::RgbaImage;
use std::path::PathBuf;

use crate::project::TextClipData;

static SANS: &[u8] = include_bytes!("../../../assets/DejaVuSans.ttf");
static SANS_BOLD: &[u8] = include_bytes!("../../../assets/DejaVuSans-Bold.ttf");

pub fn font(bold: bool) -> FontArc {
    // Unwrap is safe: the embedded fonts are validated at build time.
    FontArc::try_from_slice(if bold { SANS_BOLD } else { SANS }).expect("embedded font is valid ttf")
}

fn scaled_font(bold: bool, px: f32) -> PxScaleFont<FontArc> {
    font(bold).into_scaled(px.max(4.0))
}

/// Render a text payload into a full-frame transparent RGBA image.
///
/// * `w`/`h` — output frame size (project resolution for export; preview
///   uses the scaled preview size so text scales identically).
/// * `size_px` — font size in the same pixel space.
pub fn render_text(data: &TextClipData, w: u32, h: u32, size_px: f32) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(w.max(1), h.max(1), image::Rgba([0, 0, 0, 0]));
    let content = data.content.trim_end_matches('\n');
    if content.is_empty() {
        return img;
    }
    let fnt = scaled_font(data.bold, size_px);
    let line_h = fnt.height() * data.line_spacing.max(0.8) as f32;
    let lines: Vec<&str> = content.split('\n').collect();
    let (color, alpha) = (data.color, data.color[3]);

    // Vertical layout: y anchor is the center of the text block.
    let block_h = line_h * lines.len() as f32;
    let mut pen_y = data.y.clamp(0.0, 1.0) as f32 * h as f32 - block_h * 0.5 + line_h * 0.72;
    // 0.72 ≈ cap-height offset so the visual block centers nicely.

    for line in lines {
        let width = measure(&fnt, line);
        let anchor_x = data.x.clamp(0.0, 1.0) as f32 * w as f32;
        let mut pen_x = match data.align {
            a if a < 0.34 => anchor_x,                          // left
            a if a > 0.66 => anchor_x - width,                  // right
            _ => anchor_x - width * 0.5,                        // center
        };
        pen_x = pen_x.max(2.0).min((w.max(2) as f32 - 2.0).max(2.0));

        for ch in line.chars() {
            if ch == '\r' {
                continue;
            }
            let gid = fnt.glyph_id(ch);
            let glyph = Glyph {
                id: gid,
                scale: fnt.scale,
                position: ab_glyph::point(pen_x, pen_y),
            };
            if let Some(outlined) = fnt.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                outlined.draw(|x, y, v| {
                    let x = x as i64 + bounds.min.x as i64;
                    let y = y as i64 + bounds.min.y as i64;
                    if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                        return;
                    }
                    let px = img.get_pixel_mut(x as u32, y as u32);
                    let a = (v * alpha as f32 / 255.0) * 255.0;
                    // Blend new glyph coverage over existing (multi-line
                    // overlaps are rare but possible with tight leading).
                    let old = px.0[3] as f32;
                    let out_a = a + old * (1.0 - a / 255.0);
                    px.0 = [
                        color[0],
                        color[1],
                        color[2],
                        out_a.round().clamp(0.0, 255.0) as u8,
                    ];
                });
                pen_x += fnt.h_advance(gid);
            } else {
                // Unknown glyph (space fallback) — advance by a fixed step.
                pen_x += fnt.h_advance(fnt.glyph_id(' ')).max(size_px * 0.25);
            }
        }
        pen_y += line_h;
    }
    img
}

fn measure(fnt: &PxScaleFont<FontArc>, line: &str) -> f32 {
    line.chars().map(|c| fnt.h_advance(fnt.glyph_id(c))).sum()
}

/// Render to PNG bytes (for the export overlay input), with disk caching.
pub fn render_text_png_cached(data: &TextClipData, w: u32, h: u32, size_px: f32) -> std::io::Result<PathBuf> {
    let dir = crate::thumbnail::cache_dir().join("text");
    std::fs::create_dir_all(&dir)?;
    let key = crate::waveform::fnv64(
        format!(
            "{}|{:.1}|{:03}|{:03}|{:03}|{:03}|{}|{:.3}|{:.3}|{:.3}|{:.2}|{}x{}|{:.1}",
            data.content,
            data.size,
            data.color[0],
            data.color[1],
            data.color[2],
            data.color[3],
            data.bold,
            data.align,
            data.x,
            data.y,
            data.line_spacing,
            w,
            h,
            size_px
        )
        .as_bytes(),
    );
    let path = dir.join(format!("text-{key:x}.png"));
    if !path.is_file() {
        let img = render_text(data, w, h, size_px);
        img.save(&path)
            .map_err(std::io::Error::other)?;
    }
    Ok(path)
}
