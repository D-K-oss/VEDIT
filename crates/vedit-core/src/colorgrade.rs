//! Color grading engine for the software preview path.
//!
//! Every parametric adjustment (exposure, contrast, saturation, temperature,
//! tint) is linear/affine, so the whole grade folds into one 3×4 matrix that
//! is applied per pixel; the optional `.cube` LUT is sampled afterwards.
//! (The export path uses native ffmpeg filters — `exposure`, `eq`,
//! `colortemperature`, `colorbalance`, `lut3d` — see `export.rs`.)

use std::sync::Arc;

use crate::error::{Result, VeditError};
use crate::lut::Lut3d;
use crate::project::ColorGrade;

/// 3×4 affine transform (rows: r,g,b — each [mr, mg, mb, offset]).
#[derive(Debug, Clone)]
pub struct GradeTransform {
    matrix: [f32; 12],
    lut: Option<Arc<Lut3d>>,
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
            lut,
        }
    }

    pub fn identity() -> GradeTransform {
        GradeTransform {
            matrix: [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            lut: None,
        }
    }

    pub fn has_lut(&self) -> bool {
        self.lut.is_some()
    }

    pub fn is_identity(&self) -> bool {
        !self.has_lut() && self.matrix.iter().enumerate().all(|(i, v)| {
            let diag = i % 4 == i / 4;
            if diag { (*v - 1.0).abs() < 1e-5 } else { v.abs() < 1e-5 }
        })
    }

    /// Apply to a straight-alpha RGBA8 buffer, in place. Alpha untouched.
    pub fn apply_rgba(&self, buf: &mut [u8]) {
        if self.is_identity() {
            return;
        }
        let m = &self.matrix;
        for px in buf.chunks_exact_mut(4) {
            let r = px[0] as f32;
            let g = px[1] as f32;
            let b = px[2] as f32;
            let mut r2 = m[0] * r + m[1] * g + m[2] * b + m[3];
            let mut g2 = m[4] * r + m[5] * g + m[6] * b + m[7];
            let mut b2 = m[8] * r + m[9] * g + m[10] * b + m[11];
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
/// folded into one matrix. LUT applies afterwards (not part of the matrix).
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

/// ffmpeg filter chain equivalent of a parametric grade + LUT (export side).
/// Returns an empty string for neutral grades.
pub fn ffmpeg_grade_filters(grade: &ColorGrade) -> Result<String> {
    if grade.is_neutral() {
        return Ok(String::new());
    }
    let mut parts: Vec<String> = Vec::new();
    if grade.exposure.abs() > 1e-4 {
        parts.push(format!("exposure=exposure={:.3}", grade.exposure.clamp(-2.0, 2.0)));
    }
    let eq: Vec<String> = Vec::new();
    let _ = eq;
    let mut eq_args: Vec<String> = Vec::new();
    if grade.contrast.abs() > 1e-4 {
        eq_args.push(format!("contrast={:.3}", 1.0 + grade.contrast.clamp(-1.0, 1.0)));
    }
    if (grade.saturation - 1.0).abs() > 1e-4 {
        eq_args.push(format!("saturation={:.3}", grade.saturation.clamp(0.0, 2.0)));
    }
    if !eq_args.is_empty() {
        parts.push(format!("eq={}", eq_args.join(":")));
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
    fn ffmpeg_chain_order() {
        let g = ColorGrade {
            exposure: 0.5,
            contrast: 0.2,
            saturation: 1.5,
            temperature: 0.4,
            tint: -0.3,
            lut_path: None,
        };
        let s = ffmpeg_grade_filters(&g).unwrap();
        assert!(s.starts_with("exposure="));
        assert!(s.contains("eq=contrast=1.200:saturation=1.500"));
        assert!(s.contains("colortemperature=temperature=4900"));
        assert!(s.contains("colorbalance=gm=-0.300"));
    }
}
