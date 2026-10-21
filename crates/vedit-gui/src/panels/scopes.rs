//! Video scopes: Waveform, RGB Parade, Vectorscope and Histogram,
//! computed from the live preview frame (~8 refreshes/s).
//!
//! Venturi-style integration: two scope slots with per-slot pickers that
//! dock beside the grade controls in the Color workspace, and stay
//! available as a floating window in the Edit workspace.

use eframe::egui::{self, Color32, ComboBox, Image, RichText, TextureHandle, TextureOptions, Vec2, Window};
use serde::{Deserialize, Serialize};

use crate::app::VeditApp;
use crate::theme::Palette;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScopeKind {
    Waveform,
    Parade,
    Vectorscope,
    Histogram,
}

impl ScopeKind {
    pub const ALL: [ScopeKind; 4] = [
        ScopeKind::Waveform,
        ScopeKind::Parade,
        ScopeKind::Vectorscope,
        ScopeKind::Histogram,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ScopeKind::Waveform => "Waveform",
            ScopeKind::Parade => "RGB parade",
            ScopeKind::Vectorscope => "Vectorscope",
            ScopeKind::Histogram => "Histogram",
        }
    }

    /// Aspect of the scope drawing (w, h).
    fn aspect(&self) -> (f32, f32) {
        match self {
            ScopeKind::Vectorscope => (1.0, 1.0),
            _ => (2.35, 1.0),
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Right-docked scopes panel — the Color workspace's scope rack.
pub(crate) fn scopes_panel(app: &mut VeditApp, ctx: &egui::Context) {
    egui::SidePanel::right("scopes_panel")
        .resizable(true)
        .default_width(430.0)
        .min_width(260.0)
        .width_range(260.0..=660.0)
        .show(ctx, |ui| {
            crate::theme::panel_header(ui, "SCOPES  ·  composed frame");
            egui::ScrollArea::vertical().show(ui, |ui| {
                scopes_column(app, ctx, ui);
            });
        });
}

/// The two scope slots — docked panel used by the Color workspace.
pub(crate) fn scopes_column(app: &mut VeditApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let w = ui.available_width();
    let side_by_side = w > 620.0;
    if side_by_side {
        ui.horizontal(|ui| {
            scope_slot(app, ctx, ui, 0, (w - 12.0) / 2.0);
            scope_slot(app, ctx, ui, 1, (w - 12.0) / 2.0);
        });
    } else {
        scope_slot(app, ctx, ui, 0, w);
        ui.add_space(4.0);
        scope_slot(app, ctx, ui, 1, w);
    }
}

/// Floating scopes window (Edit workspace toggle).
pub(crate) fn scopes_window(app: &mut VeditApp, ctx: &egui::Context) {
    if !app.scopes_open {
        return;
    }
    let mut open = true;
    Window::new("Scopes")
        .open(&mut open)
        .collapsible(false)
        .default_width(680.0)
        .default_height(360.0)
        .show(ctx, |ui| {
            scopes_column(app, ctx, ui);
            ui.label(
                RichText::new("Measured from the current preview frame (composed timeline).")
                    .small()
                    .color(Palette::TEXT_FAINT),
            );
        });
    if !open {
        app.scopes_open = false;
    }
}

/// One slot: picker + scope image + graticule overlay.
fn scope_slot(app: &mut VeditApp, ctx: &egui::Context, ui: &mut egui::Ui, slot: usize, width: f32) {
    let kind = app.scope_slots[slot];
    ui.vertical(|ui| {
        // Picker row.
        ui.horizontal(|ui| {
            ComboBox::from_id_salt(format!("scope_kind_{slot}"))
                .selected_text(kind.label())
                .width(ui.available_width().min(150.0))
                .show_ui(ui, |ui| {
                    for k in ScopeKind::ALL {
                        if ui
                            .selectable_label(app.scope_slots[slot] == k, k.label())
                            .clicked()
                        {
                            app.scope_slots[slot] = k;
                            app.scopes_tex = None; // force recompute
                            app.settings.scope_slots[slot] = k;
                            app.settings.save();
                        }
                    }
                });
        });
        ui.add_space(2.0);

        // Refresh at ~8 fps or when the frame/slot changed.
        let needs = app.scopes_tex.is_none() || app.scopes_frame_no.is_multiple_of(8);
        if needs {
            app.scopes_tex = compute_slot_pair(app, ctx);
        }
        app.scopes_frame_no = app.scopes_frame_no.wrapping_add(1);

        let (aspect_w, aspect_h) = kind.aspect();
        let h = (width * aspect_h / aspect_w).clamp(120.0, 420.0);
        let tex = app
            .scopes_tex
            .as_ref()
            .map(|(a, b)| if slot == 0 { a } else { b });
        if let Some(tex) = tex {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, h), egui::Sense::hover());
            ui.put(rect, Image::new((tex.id(), Vec2::new(width, h))));
            draw_graticule(ui, rect, kind);
            let _ = resp;
        } else {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, h), egui::Sense::hover());
            ui.painter_at(rect).rect_filled(rect, 4.0, Palette::BG);
            ui.painter_at(rect).text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No preview frame — move the playhead onto a clip",
                egui::FontId::proportional(11.0),
                Palette::TEXT_FAINT,
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Texture computation
// ---------------------------------------------------------------------------

fn compute_slot_pair(app: &mut VeditApp, ctx: &egui::Context) -> Option<(TextureHandle, TextureHandle)> {
    let (fw, _fh, data) = app.scope_frame.clone()?;
    Some((
        scope_texture(ctx, app.scope_slots[0], &data, fw as usize),
        scope_texture(ctx, app.scope_slots[1], &data, fw as usize),
    ))
}

/// Dispatch one scope kind to its texture builder.
pub fn scope_texture(ctx: &egui::Context, kind: ScopeKind, frame: &[u8], frame_w: usize) -> TextureHandle {
    match kind {
        ScopeKind::Waveform => waveform_texture(ctx, frame, frame_w, false),
        ScopeKind::Parade => waveform_texture(ctx, frame, frame_w, true),
        ScopeKind::Vectorscope => vectorscope_texture(ctx, frame),
        ScopeKind::Histogram => histogram_texture(ctx, frame),
    }
}

/// Sample stride so heavy frames cost a bounded amount of work.
fn sample_step(frame: &[u8], budget_px: usize) -> usize {
    let n = frame.len() / 4;
    (n / budget_px).max(1) * 4
}

/// Waveform (luma per column) or RGB parade (three channel panels).
fn waveform_texture(ctx: &egui::Context, frame: &[u8], fw: usize, parade: bool) -> TextureHandle {
    const W: usize = 384;
    const H: usize = 164;
    const BINS: usize = 128;
    let panels = if parade { 3 } else { 1 };
    let cols = W / panels;
    let mut acc = vec![0u32; panels * cols * BINS];

    let fw = fw.max(1);
    let step = sample_step(frame, 30_000);
    let mut i = 0;
    while i + 3 < frame.len() {
        if frame[i + 3] > 32 {
            let px = (i / 4) % fw;
            #[allow(clippy::manual_is_multiple_of)]
            let col = px * cols / fw.max(1);
            let py = (i / 4) / fw;
            // Column offset per panel (parade: source x maps into its own
            // panel so horizontal structure is preserved in each channel).
            for (p, chan) in (0..panels).enumerate() {
                let v = frame[i + if parade { chan } else { 1 }];
                // Luma for the single-panel waveform.
                let val = if parade {
                    v as u32
                } else {
                    let (r, g, b) = (frame[i] as u32, frame[i + 1] as u32, frame[i + 2] as u32);
                    (2126 * r + 7152 * g + 722 * b) / 10_000
                };
                let bin = ((val as usize * BINS) / 256).min(BINS - 1);
                let idx = p * cols * BINS + col * BINS + bin;
                // Row position: py used only for density weighting (top rows
                // sampled identically, so no correction needed).
                let _ = py;
                acc[idx] += 1;
            }
        }
        i += step;
    }

    let mut img = image::RgbaImage::from_pixel(W as u32, H as u32, image::Rgba([10, 11, 10, 255]));
    // Per-panel max for a stable trace density.
    for p in 0..panels {
        let max = acc[p * cols * BINS..(p + 1) * cols * BINS]
            .iter()
            .copied()
            .max()
            .unwrap_or(1)
            .max(1) as f32;
        let tint: [u8; 3] = if parade {
            match p {
                0 => [225, 96, 96],
                1 => [110, 215, 130],
                _ => [104, 130, 235],
            }
        } else {
            [150, 235, 170]
        };
        for col in 0..cols {
            for bin in 0..BINS {
                let v = acc[p * cols * BINS + col * BINS + bin];
                if v == 0 {
                    continue;
                }
                // Venturi-style density: 1 − exp(−k·count/expected).
                let t = (1.0 - (-(v as f32) / (max * 0.28)).exp()).clamp(0.0, 1.0);
                // y: bin 0 = 0 IRE = bottom.
                let y0 = bin * H / BINS;
                let y1 = ((bin + 1) * H / BINS).min(H);
                for y in y0..y1 {
                    let py = H - 1 - y;
                    let base = img.get_pixel(col as u32 + (p * cols) as u32, py as u32).0;
                    let a = t.max(0.06);
                    let mixed = [
                        (base[0] as f32 * (1.0 - a) + tint[0] as f32 * a) as u8,
                        (base[1] as f32 * (1.0 - a) + tint[1] as f32 * a) as u8,
                        (base[2] as f32 * (1.0 - a) + tint[2] as f32 * a) as u8,
                        255,
                    ];
                    *img.get_pixel_mut(col as u32 + (p * cols) as u32, py as u32) = image::Rgba(mixed);
                }
            }
        }
    }
    // Faint 0/50/100 IRE lines baked in.
    for ire in [0usize, 64, 128] {
        let y = (H - 1 - (ire * H / 256)).min(H - 1);
        for x in 0..W {
            let px = img.get_pixel_mut(x as u32, y as u32);
            px.0 = [px.0[0].max(38), px.0[1].max(46), px.0[2].max(40), 255];
        }
    }
    ctx.load_texture(
        if parade { "scope_parade" } else { "scope_wave" },
        egui::ColorImage::from_rgba_unmultiplied([W, H], img.as_raw()),
        TextureOptions::LINEAR,
    )
}

/// Vectorscope: Cb/Cr scatter with hue targets, skin-tone line and graticule.
fn vectorscope_texture(ctx: &egui::Context, frame: &[u8]) -> TextureHandle {
    const S: usize = 220;
    let center = (S as f32 / 2.0, S as f32 / 2.0);
    let radius = S as f32 / 2.0 - 6.0;
    let mut acc = vec![0u32; S * S];

    let step = sample_step(frame, 26_000);
    let mut i = 0;
    while i + 3 < frame.len() {
        if frame[i + 3] > 32 {
            let (r, g, b) = (frame[i] as f32, frame[i + 1] as f32, frame[i + 2] as f32);
            let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            let cb = (b - y) * 0.565;
            let cr = (r - y) * 0.713;
            let x = center.0 + cb / 128.0 * radius;
            let y = center.1 - cr / 128.0 * radius;
            if x >= 0.0 && y >= 0.0 && (x as usize) < S && (y as usize) < S {
                acc[y as usize * S + x as usize] += 4;
            }
        }
        i += step;
    }

    let mut img = image::RgbaImage::from_pixel(S as u32, S as u32, image::Rgba([10, 11, 10, 255]));
    let grat = [52, 58, 52];
    // Outer circle.
    let steps = 144;
    for s in 0..steps {
        let a = s as f32 / steps as f32 * std::f32::consts::TAU;
        for t in 0..2 {
            let rr = radius - t as f32;
            let px = (center.0 + rr * a.cos()) as i32;
            let py = (center.1 + rr * a.sin()) as i32;
            if px >= 0 && py >= 0 && (px as usize) < S && (py as usize) < S {
                img.get_pixel_mut(px as u32, py as u32).0 = [grat[0], grat[1], grat[2], 255];
            }
        }
    }
    // Center cross.
    for d in -2..=2i32 {
        for (px, py) in [(center.0 as i32 + d, center.1 as i32), (center.0 as i32, center.1 as i32 + d)] {
            if px >= 0 && py >= 0 && (px as usize) < S && (py as usize) < S {
                img.get_pixel_mut(px as u32, py as u32).0 = [76, 82, 76, 255];
            }
        }
    }
    // 75% bar hue targets: (label, Cb, Cr) from 75% bars.
    let targets: [(&str, f32, f32); 6] = [
        ("Yl", -100.2, 9.8),
        ("Cy", 22.9, -107.2),
        ("G", -77.2, -97.4),
        ("Mg", 77.2, 97.4),
        ("R", -22.9, 107.2),
        ("B", 100.2, -9.8),
    ];
    let to_px = |cb: f32, cr: f32| -> (f32, f32) {
        (
            center.0 + cb / 128.0 * radius,
            center.1 - cr / 128.0 * radius,
        )
    };
    // Skin-tone line: through center toward (Cb −25, Cr +44).
    {
        let (x0, y0) = to_px(-25.0 * 2.2, 44.0 * 2.2);
        let (x1, y1) = to_px(25.0 * 2.2, -44.0 * 2.2);
        let steps = 60;
        for s in 0..=steps {
            let f = s as f32 / steps as f32;
            let px = (x0 + (x1 - x0) * f) as i32;
            let py = (y0 + (y1 - y0) * f) as i32;
            if px >= 0 && py >= 0 && (px as usize) < S && (py as usize) < S {
                img.get_pixel_mut(px as u32, py as u32).0 = [96, 72, 56, 255];
            }
        }
    }
    for (label, cb, cr) in targets {
        let (x, y) = to_px(cb, cr);
        let (xi, yi) = (x as i32, y as i32);
        for dy in -3..=3i32 {
            for dx in -3..=3i32 {
                if dx * dx + dy * dy <= 9 {
                    let px = xi + dx;
                    let py = yi + dy;
                    if px >= 0 && py >= 0 && (px as usize) < S && (py as usize) < S {
                        img.get_pixel_mut(px as u32, py as u32).0 = [64, 70, 64, 255];
                    }
                }
            }
        }
        // Label glyphs are painted by egui over the texture (crisp text).
        let _ = label;
    }
    // Scatter (green-hot trace).
    for (idx, &v) in acc.iter().enumerate() {
        if v > 0 {
            let t = (v as f32 / 14.0).sqrt().clamp(0.06, 1.0);
            let (x, y) = (idx % S, idx / S);
            img.get_pixel_mut(x as u32, y as u32).0 = [
                (90.0 + 120.0 * t) as u8,
                (150.0 + 100.0 * t) as u8,
                (120.0 + 90.0 * t) as u8,
                255,
            ];
        }
    }
    // Target labels on top.
    let mut rgba = img.as_raw().clone();
    paint_label(&mut rgba, S, "Yl", to_px(-100.2, 9.8));
    paint_label(&mut rgba, S, "Cy", to_px(22.9, -107.2));
    paint_label(&mut rgba, S, "G", to_px(-77.2, -97.4));
    paint_label(&mut rgba, S, "Mg", to_px(77.2, 97.4));
    paint_label(&mut rgba, S, "R", to_px(-22.9, 107.2));
    paint_label(&mut rgba, S, "B", to_px(100.2, -9.8));

    ctx.load_texture(
        "scope_vec",
        egui::ColorImage::from_rgba_unmultiplied([S, S], &rgba),
        TextureOptions::LINEAR,
    )
}

/// Tiny 3×5 glyph raster for vectorscope target labels.
fn paint_label(rgba: &mut [u8], size: usize, label: &str, pos: (f32, f32)) {
    const GLYPHS: &[(&str, [u8; 15])] = &[
        ("Yl", [1, 0, 1, 1, 0, 1, 1, 1, 1, 0, 1, 0, 0, 1, 0]),
        ("Cy", [0, 1, 1, 1, 0, 0, 1, 0, 0, 0, 1, 1, 0, 1, 1]),
        ("G", [0, 1, 1, 1, 0, 0, 1, 0, 1, 0, 0, 1, 1, 0, 1]),
        ("Mg", [1, 0, 1, 1, 1, 1, 1, 0, 1, 1, 0, 1, 1, 0, 1]),
        ("R", [1, 1, 0, 1, 0, 1, 1, 1, 0, 1, 0, 1, 1, 0, 1]),
        ("B", [1, 1, 0, 1, 0, 1, 1, 1, 0, 1, 0, 1, 1, 1, 0]),
    ];
    let Some((_, g)) = GLYPHS.iter().find(|(l, _)| *l == label) else { return };
    let ox = pos.0 as i32 + 4;
    let oy = pos.1 as i32 + 4;
    for (i, bit) in g.iter().enumerate() {
        if *bit == 0 {
            continue;
        }
        let gx = ox + (i % 3) as i32;
        let gy = oy + (i / 3) as i32;
        if gx >= 0 && gy >= 0 && (gx as usize) < size && (gy as usize) < size {
            let idx = (gy as usize * size + gx as usize) * 4;
            if idx + 3 < rgba.len() {
                rgba[idx] = 168;
                rgba[idx + 1] = 178;
                rgba[idx + 2] = 168;
            }
        }
    }
}

/// RGB histogram (128 bins, sqrt-scaled, channels overlaid).
fn histogram_texture(ctx: &egui::Context, frame: &[u8]) -> TextureHandle {
    const BINS: usize = 128;
    const W: usize = 384;
    const H: usize = 164;
    let mut bins = [[0u32; BINS]; 3];
    let step = sample_step(frame, 30_000);
    let mut i = 0;
    while i + 3 < frame.len() {
        if frame[i + 3] > 32 {
            for c in 0..3 {
                bins[c][(frame[i + c] >> 1) as usize % BINS] += 1;
            }
        }
        i += step;
    }
    let mut img = image::RgbaImage::from_pixel(W as u32, H as u32, image::Rgba([10, 11, 10, 255]));
    let max = bins
        .iter()
        .flat_map(|b| b.iter())
        .copied()
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    let colors: [[u8; 3]; 3] = [[225, 96, 96], [110, 215, 130], [104, 130, 235]];
    for (c, ch) in bins.iter().enumerate() {
        for (bi, &v) in ch.iter().enumerate() {
            let t = ((v as f32 / max).powf(0.5)).clamp(0.0, 1.0);
            let bh = (t * (H - 8) as f32) as usize;
            for y in 0..bh {
                let px = bi * W / BINS;
                let py = H - 2 - y;
                if px < W && py < H {
                    let base = img.get_pixel(px as u32, py as u32).0;
                    let tint = colors[c];
                    let mixed = [
                        (base[0] as f32 * 0.55 + tint[0] as f32 * 0.45) as u8,
                        (base[1] as f32 * 0.55 + tint[1] as f32 * 0.45) as u8,
                        (base[2] as f32 * 0.55 + tint[2] as f32 * 0.45) as u8,
                        255,
                    ];
                    *img.get_pixel_mut(px as u32, py as u32) = image::Rgba(mixed);
                }
            }
        }
    }
    ctx.load_texture(
        "scope_hist",
        egui::ColorImage::from_rgba_unmultiplied([W, H], img.as_raw()),
        TextureOptions::LINEAR,
    )
}

/// IRE graticule overlay for waveform / parade scopes.
fn draw_graticule(ui: &mut egui::Ui, rect: egui::Rect, kind: ScopeKind) {
    if !matches!(kind, ScopeKind::Waveform | ScopeKind::Parade) {
        return;
    }
    let p = ui.painter_at(rect);
    for ire in [25.0, 50.0, 75.0] {
        let y = rect.bottom() - (ire / 100.0) * rect.height();
        p.line_segment(
            [egui::Pos2::new(rect.left(), y), egui::Pos2::new(rect.right(), y)],
            egui::Stroke::new(1.0_f32, Color32::from_rgba_premultiplied(70, 78, 70, 110)),
        );
        p.text(
            egui::Pos2::new(rect.left() + 4.0, y - 5.0),
            egui::Align2::LEFT_TOP,
            format!("{ire}"),
            egui::FontId::monospace(8.0),
            Color32::from_rgba_premultiplied(120, 130, 120, 160),
        );
    }
}
