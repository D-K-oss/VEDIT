//! Color grading engine shared by the software preview and (as filter
//! equivalents) the export path.
//!
//! Two stages, in order:
//! 1. **Scalar grade** — exposure / contrast / saturation / temperature /
//!    tint fold into one 3×4 affine matrix applied per pixel.
//! 2. **Luma wheels** — Resolve-style Shadows / Midtones / Highlights /
//!    Offset wheels. Each wheel has a chroma offset (x, y), a Luma shift and
//!    a Saturation multiplier; the three band wheels are weighted by smooth
//!    luma masks (Shadows range / Highlights range), the Offset wheel acts
//!    on the whole image. The same math runs on the CPU (preview) and maps
//!    onto `colorbalance` + `curves` + `eq` filters (export).
//!
//! The optional `.cube` 3D LUT is sampled after everything else.

use std::sync::Arc;

use crate::error::{Result, VeditError};
use crate::lut::Lut3d;
use crate::project::{ColorGrade, Wheel};

/// 3×4 affine transform (rows: r,g,b — each [mr, mg, mb, offset]).
#[derive(Debug, Clone)]
pub struct GradeTransform {
    matrix: [f32; 12],
    wheels: Option<WheelTransform>,
    lut: Option<Arc<Lut3d>>,
}

/// Precomputed wheel setup: luma-mask ramps + the four wheels.
#[derive(Debug, Clone, Copy)]
struct WheelTransform {
    /// Shadow-mask ramp edges (luma space).
    s0: f32,
    s1: f32,
    /// Highlight-mask ramp edges.
    h0: f32,
    h1: f32,
    shadows: Wheel,
    midtones: Wheel,
    highlights: Wheel,
    offset: Wheel,
}

impl WheelTransform {
    fn build(grade: &ColorGrade) -> Option<WheelTransform> {
        let wheels_neutral = grade.shadows.is_neutral()
            && grade.midtones.is_neutral()
            && grade.highlights.is_neutral()
            && grade.offset.is_neutral();
        if wheels_neutral {
            return None;
        }
        let s_c = grade.shadows_range.clamp(0.05, 0.9) as f32;
        let h_c = grade.highlights_range.clamp(0.1, 0.98) as f32;
        Some(WheelTransform {
            s0: 0.0,
            s1: (s_c * 1.8 + 0.08).min(h_c * 0.9),
            h0: (h_c - 0.28).max(s_c * 0.9),
            h1: 1.0,
            shadows: grade.shadows,
            midtones: grade.midtones,
            highlights: grade.highlights,
            offset: grade.offset,
        })
    }

    fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
        let t = ((x - e0) / (e1 - e0).max(1e-4)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// (shadow, mid, high) mask weights at luma `l`.
    fn shares(&self, l: f32) -> (f32, f32, f32) {
        let w_shadow = 1.0 - Self::smoothstep(self.s0, self.s1.max(self.s0 + 0.01), l);
        let w_high = Self::smoothstep(self.h0.min(self.h1 - 0.01), self.h1, l);
        let w_mid = (1.0 - w_shadow - w_high).clamp(0.0, 1.0);
        (w_shadow, w_mid, w_high)
    }

    /// Effective chroma (x, y), luma and saturation at luma `l` — the band
    /// wheels weighted by their masks plus the offset wheel (weight 1).
    fn effective(&self, l: f32) -> (f32, f32, f32, f32) {
        let (ws, wm, wh) = self.shares(l);
        let (ws, wm, wh) = (ws as f64, wm as f64, wh as f64);
        let mut x = self.offset.x;
        let mut y = self.offset.y;
        let mut lum = self.offset.lum;
        let mut sat = self.offset.sat;
        x += ws * self.shadows.x + wm * self.midtones.x + wh * self.highlights.x;
        y += ws * self.shadows.y + wm * self.midtones.y + wh * self.highlights.y;
        lum += ws * self.shadows.lum + wm * self.midtones.lum + wh * self.highlights.lum;
        sat += ws * (self.shadows.sat - 1.0) + wm * (self.midtones.sat - 1.0) + wh * (self.highlights.sat - 1.0);
        (
            x.clamp(-1.0, 1.0) as f32,
            y.clamp(-1.0, 1.0) as f32,
            lum.clamp(-1.0, 1.0) as f32,
            sat.max(0.0) as f32,
        )
    }

    /// Chroma offsets (dr, dg, db) in 0..1 units for an effective (x, y).
    /// Convention: x > 0 = warm (red up, blue down), y > 0 = green — matches
    /// the project's tint sign. The export map `wheel_chroma_offsets` MUST
    /// stay in sync with this.
    fn chroma_offsets(x: f32, y: f32) -> (f32, f32, f32) {
        const K: f32 = 0.22;
        (K * (x - 0.5 * y), K * (y - 0.3 * x), -K * (x + 0.5 * y))
    }
}

impl GradeTransform {
    /// Build the transform for a grade. Parses the LUT if one is set.
    pub fn build(grade: &ColorGrade) -> Result<GradeTransform> {
        let lut = match &grade.lut_path {
            Some(p) => Some(Arc::new(Lut3d::parse_file(p)?)),
            None => None,
        };
        Ok(GradeTransform {
            matrix: build_matrix(grade),
            wheels: WheelTransform::build(grade),
            lut,
        })
    }

    /// Parse failure tolerated (missing LUT file etc.) — falls back to the
    /// parametric grade only. Used by the preview where a hard error would
    /// stall playback.
    pub fn build_lenient(grade: &ColorGrade) -> GradeTransform {
        let lut = match &grade.lut_path {
            Some(p) => Lut3d::parse_file(p).ok().map(Arc::new),
            None => None,
        };
        GradeTransform {
            matrix: build_matrix(grade),
            wheels: WheelTransform::build(grade),
            lut,
        }
    }

    pub fn identity() -> GradeTransform {
        GradeTransform {
            matrix: [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            wheels: None,
            lut: None,
        }
    }

    pub fn has_lut(&self) -> bool {
        self.lut.is_some()
    }

    pub fn is_identity(&self) -> bool {
        self.wheels.is_none()
            && !self.has_lut()
            && self.matrix.iter().enumerate().all(|(i, v)| {
                let diag = i % 4 == i / 4;
                if diag {
                    (*v - 1.0).abs() < 1e-5
                } else {
                    v.abs() < 1e-5
                }
            })
    }

    /// Apply to a straight-alpha RGBA8 buffer, in place. Alpha untouched.
    pub fn apply_rgba(&self, buf: &mut [u8]) {
        if self.is_identity() {
            return;
        }
        let m = &self.matrix;
        let wheels = self.wheels;
        for px in buf.chunks_exact_mut(4) {
            let r = px[0] as f32;
            let g = px[1] as f32;
            let b = px[2] as f32;
            let mut r2 = m[0] * r + m[1] * g + m[2] * b + m[3];
            let mut g2 = m[4] * r + m[5] * g + m[6] * b + m[7];
            let mut b2 = m[8] * r + m[9] * g + m[10] * b + m[11];

            if let Some(w) = wheels {
                let l = (0.2126 * r2 + 0.7152 * g2 + 0.0722 * b2) / 255.0;
                let (wx, wy, wlum, wsat) = w.effective(l);
                // Saturation (around luma).
                if (wsat - 1.0).abs() > 1e-3 {
                    let lum = 0.2126 * r2 + 0.7152 * g2 + 0.0722 * b2;
                    r2 = lum + (r2 - lum) * wsat;
                    g2 = lum + (g2 - lum) * wsat;
                    b2 = lum + (b2 - lum) * wsat;
                }
                // Chroma offset.
                let (dr, dg, db) = WheelTransform::chroma_offsets(wx, wy);
                r2 += dr * 255.0;
                g2 += dg * 255.0;
                b2 += db * 255.0;
                // Luma shift (additive, like a lift/gain offset).
                if wlum.abs() > 1e-3 {
                    let add = wlum * 0.35 * 255.0;
                    r2 += add;
                    g2 += add;
                    b2 += add;
                }
            }

            if let Some(lut) = &self.lut {
                let o = lut.sample(
                    (r2 / 255.0).clamp(0.0, 1.0),
                    (g2 / 255.0).clamp(0.0, 1.0),
                    (b2 / 255.0).clamp(0.0, 1.0),
                );
                r2 = o[0] * 255.0;
                g2 = o[1] * 255.0;
                b2 = o[2] * 255.0;
            }
            px[0] = r2.round().clamp(0.0, 255.0) as u8;
            px[1] = g2.round().clamp(0.0, 255.0) as u8;
            px[2] = b2.round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// 3×3 matrix + offset composition helper.
type Mat = [f32; 12];

fn ident() -> Mat {
    [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]
}

fn mul(a: &Mat, b: &Mat) -> Mat {
    // out = a · b (apply b first, then a)
    let mut o = [0.0f32; 12];
    for r in 0..3 {
        for c in 0..3 {
            o[r * 4 + c] = a[r * 4] * b[c] + a[r * 4 + 1] * b[4 + c] + a[r * 4 + 2] * b[8 + c];
        }
        o[r * 4 + 3] = a[r * 4] * b[3] + a[r * 4 + 1] * b[7] + a[r * 4 + 2] * b[11] + a[r * 4 + 3];
    }
    o
}

fn scale_rgb(r: f32, g: f32, b: f32) -> Mat {
    [
        r, 0.0, 0.0, 0.0, //
        0.0, g, 0.0, 0.0, //
        0.0, 0.0, b, 0.0,
    ]
}

/// Exposure (stops) → contrast → saturation → temperature → tint, all
/// folded into one matrix. Wheels and LUT apply afterwards.
pub fn build_matrix(grade: &ColorGrade) -> [f32; 12] {
    let mut m = ident();

    // Exposure: multiply by 2^stops.
    let ev = 2.0f32.powf(grade.exposure.clamp(-2.0, 2.0) as f32);
    m = mul(&scale_rgb(ev, ev, ev), &m);

    // Contrast around mid-gray: y = (x - 0.5) * k + 0.5.
    let k = (1.0 + grade.contrast.clamp(-1.0, 1.0)) as f32;
    let mut cm = ident();
    for r in 0..3 {
        cm[r * 4] *= k;
        cm[r * 4 + 1] *= k;
        cm[r * 4 + 2] *= k;
        cm[r * 4 + 3] = 0.5 - 0.5 * k;
    }
    m = mul(&cm, &m);

    // Saturation around Rec.709 luma:
    //   c' = luma*(1-s) + c*s,  luma = dot(rgb, [0.2126, 0.7152, 0.0722])
    let s = grade.saturation.clamp(0.0, 2.0) as f32;
    let lum = [0.2126f32, 0.7152f32, 0.0722f32];
    let mut sm = ident();
    for r in 0..3 {
        for c in 0..3 {
            sm[r * 4 + c] = if c == r {
                lum[c] + (1.0 - lum[c]) * s
            } else {
                lum[c] * (1.0 - s)
            };
        }
        sm[r * 4 + 3] = 0.0;
    }
    m = mul(&sm, &m);

    // Temperature: warm (+) boosts red, cools blue. Multiplicative gains.
    let t = grade.temperature.clamp(-1.0, 1.0) as f32;
    m = mul(&scale_rgb(1.0 + 0.18 * t, 1.0 + 0.02 * t, 1.0 - 0.18 * t), &m);

    // Tint: green (+) / magenta (-).
    let ti = grade.tint.clamp(-1.0, 1.0) as f32;
    m = mul(&scale_rgb(1.0 - 0.06 * ti, 1.0 + 0.14 * ti, 1.0 - 0.06 * ti), &m);

    m
}

/// Per-channel RGB offsets (dr, dg, db) in -1..1-ish units for one wheel —
/// MUST stay in sync with the preview mapping in `WheelTransform::
/// chroma_offsets` (x > 0 warm, y > 0 green).
pub fn wheel_chroma_offsets(w: &Wheel) -> (f64, f64, f64) {
    let k = 0.25;
    (
        (k * (w.x - 0.5 * w.y)).clamp(-1.0, 1.0),
        (k * (w.y - 0.3 * w.x)).clamp(-1.0, 1.0),
        (-k * (w.x + 0.5 * w.y)).clamp(-1.0, 1.0),
    )
}

/// ffmpeg filter chain equivalent of the full grade (scalar + wheels + LUT)
/// for the export path. Returns an empty string for neutral grades.
pub fn ffmpeg_grade_filters(grade: &ColorGrade) -> Result<String> {
    if grade.is_neutral() {
        return Ok(String::new());
    }
    let mut parts: Vec<String> = Vec::new();
    if grade.exposure.abs() > 1e-4 {
        parts.push(format!("exposure=exposure={:.3}", grade.exposure.clamp(-2.0, 2.0)));
    }

    // Scalar contrast/saturation. Wheel saturation folds into the same eq
    // (weighted — the exact per-luma masking runs preview-side).
    let sat_total = grade.saturation.clamp(0.0, 2.0)
        * (1.0 + (grade.offset.sat - 1.0)
            + 0.3 * (grade.shadows.sat - 1.0)
            + 0.4 * (grade.midtones.sat - 1.0)
            + 0.3 * (grade.highlights.sat - 1.0))
            .clamp(0.0, 2.0);

    let mut eq_args: Vec<String> = Vec::new();
    if grade.contrast.abs() > 1e-4 {
        eq_args.push(format!("contrast={:.3}", 1.0 + grade.contrast.clamp(-1.0, 1.0)));
    }
    if (sat_total - 1.0).abs() > 1e-3 {
        eq_args.push(format!("saturation={:.3}", sat_total.clamp(0.0, 3.0)));
    }
    if !eq_args.is_empty() {
        parts.push(format!("eq={}", eq_args.join(":")));
    }

    // Wheels → colorbalance (per-band RGB) + curves (per-band luma).
    let wheels_active = !grade.shadows.is_neutral()
        || !grade.midtones.is_neutral()
        || !grade.highlights.is_neutral()
        || !grade.offset.is_neutral();
    if wheels_active {
        let mut cb: Vec<String> = Vec::new();
        // colorbalance options are channel-band: rs/gs/bs (shadows),
        // rm/gm/bm (midtones), rh/gh/bh (highlights).
        let push_band = |r_key: &str, g_key: &str, b_key: &str, w: &Wheel, cb: &mut Vec<String>| {
            if w.is_neutral() {
                return;
            }
            let (dr, dg, db) = wheel_chroma_offsets(w);
            cb.push(format!("{r_key}={:.3}", dr.clamp(-1.0, 1.0)));
            cb.push(format!("{g_key}={:.3}", dg.clamp(-1.0, 1.0)));
            cb.push(format!("{b_key}={:.3}", db.clamp(-1.0, 1.0)));
        };
        push_band("rs", "gs", "bs", &grade.shadows, &mut cb);
        push_band("rm", "gm", "bm", &grade.midtones, &mut cb);
        push_band("rh", "gh", "bh", &grade.highlights, &mut cb);
        if !grade.offset.is_neutral() {
            // Offset wheel spreads across all three bands at reduced strength.
            let off = grade.offset;
            for (rk, gk, bk, share) in
                [("rs", "gs", "bs", 0.6), ("rm", "gm", "bm", 1.0), ("rh", "gh", "bh", 0.6)]
            {
                let scaled = Wheel { x: off.x * share, y: off.y * share, lum: 0.0, sat: 1.0 };
                let (dr, dg, db) = wheel_chroma_offsets(&scaled);
                cb.push(format!("{rk}={:.3}", dr.clamp(-1.0, 1.0)));
                cb.push(format!("{gk}={:.3}", dg.clamp(-1.0, 1.0)));
                cb.push(format!("{bk}={:.3}", db.clamp(-1.0, 1.0)));
            }
        }
        if !cb.is_empty() {
            parts.push(format!("colorbalance={}", cb.join(":")));
        }

        // Luma wheels → master curve with three control points (kept strictly
        // monotonic).
        let ls = 0.0 + 0.35 * grade.shadows.lum.clamp(-1.0, 1.0);
        let lm = 0.5 + 0.35 * grade.midtones.lum.clamp(-1.0, 1.0) + 0.15 * grade.offset.lum.clamp(-1.0, 1.0);
        let lh = 1.0 + 0.35 * grade.highlights.lum.clamp(-1.0, 1.0);
        let mut pts = [(0.0f64, ls.clamp(0.0, 0.45)), (0.5f64, lm.clamp(0.05, 0.95)), (1.0f64, lh.clamp(0.55, 1.0))];
        for i in 1..pts.len() {
            if pts[i].1 <= pts[i - 1].1 {
                pts[i].1 = (pts[i - 1].1 + 0.01).min(1.0);
            }
        }
        if (pts[0].1 - 0.0).abs() > 1e-3
            || (pts[1].1 - 0.5).abs() > 1e-3
            || (pts[2].1 - 1.0).abs() > 1e-3
        {
            parts.push(format!(
                "curves=master='0/{:.3} 0.5/{:.3} 1/{:.3}'",
                pts[0].1, pts[1].1, pts[2].1
            ));
        }
    }

    if grade.temperature.abs() > 1e-4 {
        // User range -1 (cool) .. +1 (warm) → Kelvin 10 500 .. 2 500.
        let kelvin = (6500.0 - grade.temperature.clamp(-1.0, 1.0) * 4000.0).round();
        parts.push(format!("colortemperature=temperature={:.0}", kelvin));
    }
    if grade.tint.abs() > 1e-4 {
        parts.push(format!("colorbalance=gm={:.3}", grade.tint.clamp(-1.0, 1.0)));
    }
    if let Some(p) = &grade.lut_path {
        let path = p
            .to_str()
            .ok_or_else(|| VeditError::Other("LUT path is not valid UTF-8".into()))?;
        if !p.is_file() {
            return Err(VeditError::Other(format!("LUT file not found: {path}")));
        }
        parts.push(format!("lut3d=file={}", escape_filter_path(path)));
    }
    Ok(parts.join(","))
}

/// Escape a filesystem path for use inside an ffmpeg filter argument.
fn escape_filter_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 8);
    out.push('\'');
    for c in path.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            ':' => out.push_str("\\:"),
            _ => out.push(c),
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_is_identity() {
        let g = ColorGrade::default();
        assert!(g.is_neutral());
        let t = GradeTransform::build_lenient(&g);
        assert!(t.is_identity());
        let mut px = [100u8, 150, 200, 255];
        t.apply_rgba(&mut px);
        assert_eq!(px, [100, 150, 200, 255]);
    }

    #[test]
    fn exposure_brightens() {
        let g = ColorGrade { exposure: 1.0, ..Default::default() };
        let t = GradeTransform::build_lenient(&g);
        let mut px = [50u8, 50, 50, 255];
        t.apply_rgba(&mut px);
        assert!(px[0] > 95, "exposure +1 stop should roughly double 50 → {} ", px[0]);
    }

    #[test]
    fn saturation_desaturates() {
        let g = ColorGrade { saturation: 0.0, ..Default::default() };
        let t = GradeTransform::build_lenient(&g);
        let mut px = [200u8, 100, 50, 255];
        t.apply_rgba(&mut px);
        let luma = 0.2126 * 200.0 + 0.7152 * 100.0 + 0.0722 * 50.0;
        assert!((px[0] as f32 - luma).abs() < 2.0);
        assert_eq!(px[0], px[1], "all channels equal to luma when s=0");
    }

    #[test]
    fn shadow_wheel_warms_blacks_only() {
        let g = ColorGrade {
            shadows: Wheel { x: 0.8, y: 0.0, lum: 0.0, sat: 1.0 },
            ..Default::default()
        };
        assert!(!g.is_neutral());
        let t = GradeTransform::build_lenient(&g);
        let mut black = [8u8, 8, 8, 255];
        t.apply_rgba(&mut black);
        assert!(black[0] > black[2] + 3, "shadow x>0 should push blacks warm: {black:?}");
        let mut white = [245u8, 245, 245, 255];
        let before = white;
        t.apply_rgba(&mut white);
        assert!(
            (white[0] as i32 - white[2] as i32).abs() <= (before[0] as i32 - before[2] as i32).abs() + 6,
            "highlights barely move: {white:?}"
        );
    }

    #[test]
    fn wheel_lum_lifts_blacks() {
        let g = ColorGrade {
            shadows: Wheel { x: 0.0, y: 0.0, lum: 0.5, sat: 1.0 },
            ..Default::default()
        };
        let t = GradeTransform::build_lenient(&g);
        let mut px = [0u8, 0, 0, 255];
        t.apply_rgba(&mut px);
        assert!(px[0] > 20, "shadow lum should lift blacks: {px:?}");
    }

    #[test]
    fn ffmpeg_chain_order() {
        let g = ColorGrade {
            exposure: 0.5,
            contrast: 0.2,
            saturation: 1.5,
            temperature: 0.4,
            tint: -0.3,
            ..Default::default()
        };
        let s = ffmpeg_grade_filters(&g).unwrap();
        assert!(s.starts_with("exposure="));
        assert!(s.contains("eq=contrast=1.200:saturation=1.500"));
        assert!(s.contains("colortemperature=temperature=4900"));
        assert!(s.contains("colorbalance=gm=-0.300"));
    }

    #[test]
    fn ffmpeg_wheels_use_colorbalance() {
        let g = ColorGrade {
            shadows: Wheel { x: 0.5, y: 0.0, lum: 0.2, sat: 1.0 },
            ..Default::default()
        };
        let s = ffmpeg_grade_filters(&g).unwrap();
        assert!(s.contains("colorbalance=rs="), "{s}");
        assert!(s.contains("curves=master="), "{s}");
    }
}
