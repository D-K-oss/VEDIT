//! `.cube` 3D LUT parsing + sampling.
//!
//! Supports the standard Resolve/IRIDAS `.cube` text format: `LUT_3D_SIZE N`
//! followed by `N^3` RGB float triplets (fastest varying = R). 1D LUTs are
//! rejected (not used by the editor).

use std::path::Path;

use crate::error::{Result, VeditError};

/// A parsed 3D LUT. Entries are laid out with R fastest-varying, as in the
/// `.cube` format itself.
#[derive(Debug, Clone)]
pub struct Lut3d {
    pub size: usize,
    /// `size^3` RGB triplets in linear 0..1 space.
    pub data: Vec<[f32; 3]>,
}

impl Lut3d {
    pub fn parse_file(path: &Path) -> Result<Lut3d> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| VeditError::Other(format!("cannot read LUT {}: {e}", path.display())))?;
        Self::parse(&text)
            .ok_or_else(|| VeditError::Other(format!("invalid or unsupported .cube file: {}", path.display())))
    }

    /// Parse `.cube` text. Returns `None` for 1D LUTs or malformed data.
    pub fn parse(text: &str) -> Option<Lut3d> {
        let mut size: Option<usize> = None;
        let mut data: Vec<[f32; 3]> = Vec::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut it = line.split_whitespace();
            let first = it.next()?;
            match first {
                "TITLE" => continue,
                "LUT_1D_SIZE" => return None, // 1D LUTs unsupported
                "LUT_3D_SIZE" => {
                    let n: usize = it.next()?.parse().ok()?;
                    if !(2..=256).contains(&n) {
                        return None;
                    }
                    size = Some(n);
                }
                "DOMAIN_MIN" | "DOMAIN_MAX" | "LUT_3D_INPUT_RANGE" => continue,
                _ => {
                    // Data line: "R G B" floats.
                    let r: f32 = first.parse().ok()?;
                    let g: f32 = it.next()?.parse().ok()?;
                    let b: f32 = it.next()?.parse().ok()?;
                    if it.next().is_some() {
                        return None;
                    }
                    data.push([r, g, b]);
                }
            }
        }
        let size = size?;
        if data.len() != size * size * size {
            return None;
        }
        Some(Lut3d { size, data })
    }

    #[inline]
    fn idx(&self, r: usize, g: usize, b: usize) -> usize {
        // .cube layout: R fastest, then G, then B.
        (b * self.size + g) * self.size + r
    }

    /// Trilinear sample. Input clamped to 0..1.
    pub fn sample(&self, r: f32, g: f32, b: f32) -> [f32; 3] {
        let n = self.size.max(2) as f32;
        let f = |v: f32| -> (usize, usize, f32) {
            let x = v.clamp(0.0, 1.0) * (n - 1.0);
            let i = x.floor() as usize;
            (i.min(self.size - 1), (i + 1).min(self.size - 1), x - x.floor())
        };
        let (r0, r1, fr) = f(r);
        let (g0, g1, fg) = f(g);
        let (b0, b1, fb) = f(b);
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let c00 = [
            lerp(self.data[self.idx(r0, g0, b0)][0], self.data[self.idx(r1, g0, b0)][0], fr),
            lerp(self.data[self.idx(r0, g0, b0)][1], self.data[self.idx(r1, g0, b0)][1], fr),
            lerp(self.data[self.idx(r0, g0, b0)][2], self.data[self.idx(r1, g0, b0)][2], fr),
        ];
        let c10 = [
            lerp(self.data[self.idx(r0, g1, b0)][0], self.data[self.idx(r1, g1, b0)][0], fr),
            lerp(self.data[self.idx(r0, g1, b0)][1], self.data[self.idx(r1, g1, b0)][1], fr),
            lerp(self.data[self.idx(r0, g1, b0)][2], self.data[self.idx(r1, g1, b0)][2], fr),
        ];
        let c01 = [
            lerp(self.data[self.idx(r0, g0, b1)][0], self.data[self.idx(r1, g0, b1)][0], fr),
            lerp(self.data[self.idx(r0, g0, b1)][1], self.data[self.idx(r1, g0, b1)][1], fr),
            lerp(self.data[self.idx(r0, g0, b1)][2], self.data[self.idx(r1, g0, b1)][2], fr),
        ];
        let c11 = [
            lerp(self.data[self.idx(r0, g1, b1)][0], self.data[self.idx(r1, g1, b1)][0], fr),
            lerp(self.data[self.idx(r0, g1, b1)][1], self.data[self.idx(r1, g1, b1)][1], fr),
            lerp(self.data[self.idx(r0, g1, b1)][2], self.data[self.idx(r1, g1, b1)][2], fr),
        ];
        let c0 = [
            lerp(c00[0], c10[0], fg),
            lerp(c00[1], c10[1], fg),
            lerp(c00[2], c10[2], fg),
        ];
        let c1 = [
            lerp(c01[0], c11[0], fg),
            lerp(c01[1], c11[1], fg),
            lerp(c01[2], c11[2], fg),
        ];
        [
            lerp(c0[0], c1[0], fb),
            lerp(c0[1], c1[1], fb),
            lerp(c0[2], c1[2], fb),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_cube() {
        let text = "TITLE t\nLUT_3D_SIZE 2\n\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let lut = Lut3d::parse(text).expect("parses");
        assert_eq!(lut.size, 2);
        assert_eq!(lut.data.len(), 8);
        let out = lut.sample(0.0, 0.0, 0.0);
        assert!((out[0] - 0.0).abs() < 1e-5);
        let out = lut.sample(1.0, 1.0, 1.0);
        assert!((out[0] - 1.0).abs() < 1e-5 && (out[2] - 1.0).abs() < 1e-5);
        // Midpoint interpolates.
        let out = lut.sample(0.5, 0.5, 0.5);
        assert!((out[0] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn rejects_1d() {
        assert!(Lut3d::parse("LUT_1D_SIZE 2\n0 0 0\n1 1 1\n").is_none());
    }
}
