//! Geometric warp + blend-mode compositing for the software preview.
//!
//! `warp_into` maps a source RGBA frame into a full-size output canvas with
//! crop → flip → scale → rotate → translate (inverse-mapped bilinear
//! sampling, alpha 0 outside). `blend_over` composites that canvas with one
//! of the project blend modes. The export path produces the same result with
//! ffmpeg `crop/rotate/scale/overlay/blend` filters.

use crate::project::{BlendMode, Crop};

/// A resolved per-frame transform (values already evaluated at the frame time).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameTransform {
    pub scale_x: f32,
    pub scale_y: f32,
    /// Position offset as a fraction of the output size (0 = centered).
    pub pos_x: f32,
    pub pos_y: f32,
    /// Degrees, clockwise.
    pub rotation: f32,
    /// Rotation center as a fraction of the frame (0.5 = center).
    pub pivot_x: f32,
    pub pivot_y: f32,
    pub flip_h: bool,
    pub flip_v: bool,
}

impl FrameTransform {
    pub fn identity() -> Self {
        FrameTransform {
            scale_x: 1.0,
            scale_y: 1.0,
            pos_x: 0.0,
            pos_y: 0.0,
            rotation: 0.0,
            pivot_x: 0.5,
            pivot_y: 0.5,
            flip_h: false,
            flip_v: false,
        }
    }

    pub fn is_identity(&self) -> bool {
        *self == FrameTransform::identity()
    }
}

/// Crop a straight-alpha RGBA8 buffer in place. Fractions are of each edge.
pub fn crop_rgba(src: &mut Vec<u8>, w: u32, h: u32, crop: Crop) -> (u32, u32) {
    if crop.is_identity() || !crop.is_valid() || w == 0 || h == 0 {
        return (w, h);
    }
    let l = (crop.left.clamp(0.0, 0.95) * w as f64).round() as u32;
    let r = (crop.right.clamp(0.0, 0.95) * w as f64).round() as u32;
    let t = (crop.top.clamp(0.0, 0.95) * h as f64).round() as u32;
    let b = (crop.bottom.clamp(0.0, 0.95) * h as f64).round() as u32;
    let nw = w.saturating_sub(l + r).max(2);
    let nh = h.saturating_sub(t + b).max(2);
    let mut out = Vec::with_capacity(nw as usize * nh as usize * 4);
    for y in t..t + nh {
        let row = &src[(y as usize) * w as usize * 4..];
        out.extend_from_slice(&row[(l as usize) * 4..(l + nw) as usize * 4]);
    }
    *src = out;
    (nw, nh)
}

/// Warp `src` (straight-alpha RGBA8, `sw×sh`) into `out` (`ow×oh`) applying
/// crop (already applied if desired via `crop_rgba`), flip, scale, rotate and
/// translate. Pixels outside the source become transparent.
pub fn warp_into(
    out: &mut [u8],
    ow: u32,
    oh: u32,
    src: &[u8],
    sw: u32,
    sh: u32,
    t: FrameTransform,
) {
    if sw == 0 || sh == 0 || ow == 0 || oh == 0 {
        return;
    }
    if t.is_identity() && ow == sw && oh == sh {
        // Fast path: 1:1 blit.
        let n = (ow as usize * oh as usize * 4).min(src.len());
        out[..n].copy_from_slice(&src[..n]);
        return;
    }

    let rad = t.rotation as f64 * std::f64::consts::PI / 180.0;
    let (sin_r, cos_r) = rad.sin_cos();
    let sx = t.scale_x.max(0.01) as f64;
    let sy = t.scale_y.max(0.01) as f64;

    // Output-space center (scaled/positioned around), and the rotation
    // center (which honors the pivot).
    let cx = ow as f64 / 2.0 + t.pos_x as f64 * ow as f64;
    let cy = oh as f64 / 2.0 + t.pos_y as f64 * oh as f64;
    let rx0 = cx + (t.pivot_x as f64 - 0.5) * ow as f64;
    let ry0 = cy + (t.pivot_y as f64 - 0.5) * oh as f64;

    // Inverse map: out → (undo rotation about pivot) → (undo scale about
    // center) → (undo flip) → source pixel.
    let swf = sw as f64;
    let shf = sh as f64;
    for oy in 0..oh {
        let dy = oy as f64 + 0.5 - ry0;
        let mut ox = 0;
        let row = &mut out[(oy as usize) * ow as usize * 4..(oy as usize + 1) * ow as usize * 4];
        for px in row.chunks_exact_mut(4) {
            let dx = ox as f64 + 0.5 - rx0;
            ox += 1;
            // Rotate back about the pivot, then re-anchor to the center.
            let rx = dx * cos_r + dy * sin_r + (rx0 - cx);
            let ry = -dx * sin_r + dy * cos_r + (ry0 - cy);
            // Scale back (about the source center).
            let fx = rx / sx + swf / 2.0;
            let fy = ry / sy + shf / 2.0;
            // Flip: source x = sw - 1 - fx when flipped.
            let (sx_f, sy_f) = if t.flip_h { (swf - fx, fy) } else { (fx, fy) };
            let (sx_f, sy_f) = if t.flip_v { (sx_f, shf - sy_f) } else { (sx_f, sy_f) };
            if sx_f < 0.0 || sy_f < 0.0 || sx_f >= swf || sy_f >= shf {
                px[3] = 0; // transparent outside
                continue;
            }
            let x0 = sx_f.floor() as u32;
            let y0 = sy_f.floor() as u32;
            let x1 = (x0 + 1).min(sw - 1);
            let y1 = (y0 + 1).min(sh - 1);
            let fx = sx_f - x0 as f64;
            let fy = sy_f - y0 as f64;
            let p00 = &src[(y0 as usize * sw as usize + x0 as usize) * 4..];
            let p10 = &src[(y0 as usize * sw as usize + x1 as usize) * 4..];
            let p01 = &src[(y1 as usize * sw as usize + x0 as usize) * 4..];
            let p11 = &src[(y1 as usize * sw as usize + x1 as usize) * 4..];
            for c in 0..4 {
                let top = p00[c] as f64 * (1.0 - fx) + p10[c] as f64 * fx;
                let bot = p01[c] as f64 * (1.0 - fx) + p11[c] as f64 * fx;
                px[c] = (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// One-pixel blend functions on 0..1 f32, straight alpha handled by the caller.
#[inline]
fn blend_pixel(mode: BlendMode, d: f32, s: f32) -> f32 {
    match mode {
        BlendMode::Add => (d + s).clamp(0.0, 1.0),
        BlendMode::Multiply => d * s,
        BlendMode::Screen => d + s - d * s,
        BlendMode::Overlay => {
            if d < 0.5 {
                2.0 * d * s
            } else {
                1.0 - 2.0 * (1.0 - d) * (1.0 - s)
            }
        }
        BlendMode::SoftLight => {
            // Pegtop approximation.
            (1.0 - 2.0 * s) * d * d + 2.0 * s * d
        }
        BlendMode::Normal => s,
    }
}

/// Composite `src` over `dst` (both straight-alpha RGBA8, same size) with a
/// blend mode + opacity. `Normal` is the classic alpha-over.
pub fn blend_over_mode(dst: &mut [u8], src: &[u8], mode: BlendMode, opacity: f32) {
    let op = opacity.clamp(0.0, 1.0);
    if op <= 0.0 {
        return;
    }
    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let sa = (s[3] as f32 / 255.0) * op;
        if sa <= 0.001 {
            continue;
        }
        if mode == BlendMode::Normal {
            let da = d[3] as f32 / 255.0;
            let out_a = sa + da * (1.0 - sa);
            if out_a > 0.001 {
                for i in 0..3 {
                    let mix = (s[i] as f32 * sa + d[i] as f32 * da * (1.0 - sa)) / out_a;
                    d[i] = mix.round().clamp(0.0, 255.0) as u8;
                }
                d[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
            }
            continue;
        }
        // Non-normal modes: result = mix(dst, mode(dst, src), src_alpha).
        let da = d[3] as f32 / 255.0;
        for i in 0..3 {
            let df = d[i] as f32 / 255.0;
            let sf = s[i] as f32 / 255.0;
            let blended = blend_pixel(mode, df, sf).clamp(0.0, 1.0);
            let out = df * (1.0 - sa) + blended * sa;
            d[i] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        let out_a = (sa + da * (1.0 - sa)).clamp(0.0, 1.0);
        d[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
    }
}

/// Composite `src` over `dst` with plain alpha-over (kept for call sites
/// that don't need blend modes).
pub fn blend_over(dst: &mut [u8], src: &[u8], opacity: f32) {
    blend_over_mode(dst, src, BlendMode::Normal, opacity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_cuts_edges() {
        // 4×2 red frame, crop 25% left/right.
        let mut src = vec![255u8, 0, 0, 255].repeat(4 * 2);
        let (w, h) = crop_rgba(&mut src, 4, 2, Crop { left: 0.25, right: 0.25, top: 0.0, bottom: 0.0 });
        assert_eq!((w, h), (2, 2));
        assert_eq!(src.len(), 2 * 2 * 4);
    }

    #[test]
    fn warp_identity_is_copy() {
        let src = vec![200u8; 4 * 4 * 4];
        let mut out = vec![0u8; 4 * 4 * 4];
        warp_into(&mut out, 4, 4, &src, 4, 4, FrameTransform::identity());
        assert!(out.iter().all(|&v| v == 200));
    }

    #[test]
    fn warp_scale_down_shrinks() {
        let mut out = vec![0u8; 8 * 8 * 4];
        let src = vec![255u8, 0, 0, 255].repeat(4 * 4);
        warp_into(
            &mut out,
            8,
            8,
            &src,
            4,
            4,
            FrameTransform { scale_x: 0.5, scale_y: 0.5, ..FrameTransform::identity() },
        );
        let center = &out[(4 * 8 + 4) * 4..(4 * 8 + 4) * 4 + 4];
        assert_eq!(center[0], 255, "center should be red");
        assert_eq!(center[3], 255);
        // Corner stays outside → transparent.
        let corner = &out[0..4];
        assert_eq!(corner[3], 0);
    }

    #[test]
    fn blend_modes() {
        let dst = vec![100u8, 100, 100, 255];
        let src = vec![200u8, 200, 200, 255];
        let mut d = dst.clone();
        blend_over_mode(&mut d, &src, BlendMode::Add, 1.0);
        assert_eq!(d[0], 255);
        let mut d = dst.clone();
        blend_over_mode(&mut d, &src, BlendMode::Multiply, 1.0);
        assert!(d[0] < 90, "multiply darkens: {}", d[0]);
        let mut d = dst.clone();
        blend_over_mode(&mut d, &src, BlendMode::Screen, 1.0);
        assert!(d[0] > 215, "screen brightens (100,200 → 222): {}", d[0]);
    }
}
