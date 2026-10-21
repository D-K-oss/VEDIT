//! Scopes window: RGB histogram + vectorscope, computed from the live
//! preview frame (downsampled, refreshed ~6×/s).

use eframe::egui::{self, Image, RichText, TextureHandle, TextureOptions, Vec2};

use crate::app::VeditApp;
use crate::theme::Palette;

pub(crate) fn scopes(app: &mut VeditApp, ctx: &egui::Context) {
    if !app.scopes_open {
        return;
    }
    let mut open = true;
    egui::Window::new("Scopes")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            // Refresh at ~6 fps.
            let needs = app.scopes_tex.is_none() || app.scopes_frame_no % 10 == 0;
            if needs {
                app.scopes_tex = compute_scopes_textures(app, ctx);
            }
            app.scopes_frame_no = app.scopes_frame_no.wrapping_add(1);

            if let Some((hist, vec)) = &app.scopes_tex {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Histogram").small().color(Palette::TEXT_DIM));
                        ui.add(Image::new((hist.id(), Vec2::new(256.0, 150.0))).max_size(Vec2::new(248.0, 150.0)));
                    });
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Vectorscope").small().color(Palette::TEXT_DIM));
                        ui.add(Image::new((vec.id(), Vec2::new(150.0, 150.0))).max_size(Vec2::new(150.0, 150.0)));
                    });
                });
            } else {
                ui.label(
                    RichText::new("No preview frame yet — move the playhead onto a clip.")
                        .small()
                        .color(Palette::TEXT_DIM),
                );
            }
            ui.label(
                RichText::new("Computed from the current preview frame.")
                    .small()
                    .color(Palette::TEXT_DIM),
            );
        });
    if !open {
        app.scopes_open = false;
    }
}

fn compute_scopes_textures(app: &mut VeditApp, ctx: &egui::Context) -> Option<(TextureHandle, TextureHandle)> {
    let frame = app.scope_frame.clone()?;
    Some((histogram_texture(ctx, &frame.2), vectorscope_texture(ctx, &frame.2)))
}

/// RGB histogram texture (128 bins, sqrt-scaled columns).
fn histogram_texture(ctx: &egui::Context, frame: &[u8]) -> TextureHandle {
    const BINS: usize = 128;
    let mut bins = [[0u32; BINS]; 3];
    let n = frame.len() / 4;
    let stride_px = (n / 24_000).max(1);
    let step = stride_px * 4;
    let mut i = 0;
    while i + 3 < frame.len() {
        if frame[i + 3] > 32 {
            for c in 0..3 {
                bins[c][(frame[i + c] >> 1) as usize % BINS] += 1;
            }
        }
        i += step;
    }
    let w = 256usize;
    let h = 150usize;
    let mut img = image::RgbaImage::from_pixel(w as u32, h as u32, image::Rgba([14, 14, 16, 255]));
    let max = bins
        .iter()
        .flat_map(|b| b.iter())
        .copied()
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    let colors: [[u8; 3]; 3] = [[220, 90, 90], [90, 200, 120], [90, 120, 220]];
    for (c, bins) in bins.iter().enumerate() {
        for (bi, &v) in bins.iter().enumerate() {
            let t = ((v as f32 / max).powf(0.5)).clamp(0.0, 1.0);
            let bh = (t * (h - 10) as f32) as usize;
            for y in 0..bh {
                let px = bi * w / BINS;
                let py = h - 2 - y;
                if px < w && py < h {
                    let base = colors[c].map(|v| (v as f32 * 0.65) as u8);
                    img.get_pixel_mut(px as u32, py as u32).0 = [base[0], base[1], base[2], 255];
                }
            }
        }
    }
    ctx.load_texture(
        "scopes_hist",
        egui::ColorImage::from_rgba_unmultiplied([w, h], img.as_raw()),
        TextureOptions::LINEAR,
    )
}

/// Vectorscope: Cb/Cr scatter around a hue graticule.
fn vectorscope_texture(ctx: &egui::Context, frame: &[u8]) -> TextureHandle {
    const S: usize = 150;
    let center = (S as f32 / 2.0, S as f32 / 2.0);
    let radius = S as f32 / 2.0 - 4.0;
    let mut acc = vec![0u32; S * S];

    let n = frame.len() / 4;
    let stride_px = (n / 20_000).max(1);
    let step = stride_px * 4;
    let mut i = 0;
    while i + 3 < frame.len() {
        if frame[i + 3] > 32 {
            let (r, g, b) = (frame[i] as f32, frame[i + 1] as f32, frame[i + 2] as f32);
            let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            let cb = (b - y) * 0.565;
            let cr = (r - y) * 0.713;
            // Map: x = Cb (−128..128 → −R..R), y = −Cr (green down? standard:
            // up = red/magenta, left = green/blue… we use up = +Cr).
            let x = center.0 + cb / 128.0 * radius;
            let y = center.1 - cr / 128.0 * radius;
            if x >= 0.0 && y >= 0.0 && (x as usize) < S && (y as usize) < S {
                acc[y as usize * S + x as usize] += 4;
            }
        }
        i += step;
    }

    let mut img = image::RgbaImage::from_pixel(S as u32, S as u32, image::Rgba([12, 12, 14, 255]));
    // Graticule: outer circle + hue ticks.
    let graticule = Palette::LINE;
    let steps = 72;
    for s in 0..steps {
        let a = s as f32 / steps as f32 * std::f32::consts::TAU;
        for t in 0..2 {
            let rr = radius - t as f32;
            let px = (center.0 + rr * a.cos()) as i32;
            let py = (center.1 + rr * a.sin()) as i32;
            if px >= 0 && py >= 0 && (px as usize) < S && (py as usize) < S {
                img.get_pixel_mut(px as u32, py as u32).0 = [graticule.r(), graticule.g(), graticule.b(), 255];
            }
        }
    }
    // Center cross.
    for d in -2..=2i32 {
        let px = center.0 as i32 + d;
        let py = center.1 as i32 + d;
        if px >= 0 && py >= 0 && (px as usize) < S && (py as usize) < S {
            img.get_pixel_mut(px as u32, py as u32).0 = [70, 70, 76, 255];
        }
    }
    // Scatter (white-hot accumulation).
    for (idx, &v) in acc.iter().enumerate() {
        if v > 0 {
            let t = (v as f32 / 12.0).sqrt().clamp(0.05, 1.0);
            let (x, y) = (idx % S, idx / S);
            img.get_pixel_mut(x as u32, y as u32).0 = [
                (110.0 + 130.0 * t) as u8,
                (170.0 + 60.0 * t) as u8,
                (150.0 + 80.0 * t) as u8,
                255,
            ];
        }
    }
    ctx.load_texture(
        "scopes_vec",
        egui::ColorImage::from_rgba_unmultiplied([S, S], img.as_raw()),
        TextureOptions::LINEAR,
    )
}
