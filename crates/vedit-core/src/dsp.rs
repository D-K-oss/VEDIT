//! Real-time audio DSP for the mixer: parametric EQ + multiband compressor.
//!
//! The *configuration* types (`EqState`, `CompState`) are part of the project
//! model and serialize to `.vedit.json`. The *runtime* types (`EqRuntime`,
//! `CompRuntime`) hold filter state and process samples on the audio thread —
//! they are also used to evaluate the EQ response curve for the editor UI.
//!
//! Everything is mono-per-channel: process the left and right channel with
//! separate runtime instances.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Biquad (transposed direct form II)
// ---------------------------------------------------------------------------

/// One second-order section. Coefficients are recomputed whenever a band
/// parameter changes (cheap — a handful of trig ops).
#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
    active: bool,
}

impl Biquad {
    pub fn bypass() -> Self {
        Biquad { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: 0.0, z2: 0.0, active: false }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    pub fn process(&mut self, x: f32) -> f32 {
        if !self.active {
            return x;
        }
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    /// Magnitude response in dB at `freq` Hz (0 dB when bypassed).
    pub fn magnitude_db(&self, freq: f64, sample_rate: f64) -> f64 {
        if !self.active {
            return 0.0;
        }
        let w = 2.0 * std::f64::consts::PI * freq / sample_rate;
        let (cw, sw) = (w.cos(), w.sin());
        let cw2 = (2.0 * w).cos();
        let sw2 = (2.0 * w).sin();
        let (b0, b1, b2, a0, a1, a2) =
            (self.b0 as f64, self.b1 as f64, self.b2 as f64, 1.0, self.a1 as f64, self.a2 as f64);
        let num_re = b0 + b1 * cw + b2 * cw2;
        let num_im = -(b1 * sw + b2 * sw2);
        let den_re = a0 + a1 * cw + a2 * cw2;
        let den_im = -(a1 * sw + a2 * sw2);
        let num = (num_re * num_re + num_im * num_im).sqrt();
        let den = (den_re * den_re + den_im * den_im).sqrt().max(1e-12);
        20.0 * (num / den).log10()
    }
}

fn design(kind: BandKind, freq: f64, gain_db: f64, q: f64, sample_rate: f64) -> Biquad {
    let fs = sample_rate.max(8_000.0);
    let f0 = freq.clamp(10.0, fs * 0.49);
    let q = q.max(0.05);
    let w0 = 2.0 * std::f64::consts::PI * f0 / fs;
    let (s, c) = w0.sin_cos();
    let alpha = s / (2.0 * q);
    let a0;
    let (mut b0, mut b1, mut b2, mut a1, mut a2);

    match kind {
        BandKind::LowCut => {
            b0 = (1.0 + c) / 2.0;
            b1 = -(1.0 + c);
            b2 = (1.0 + c) / 2.0;
            a0 = 1.0 + alpha;
            a1 = -2.0 * c;
            a2 = 1.0 - alpha;
        }
        BandKind::HighCut => {
            b0 = (1.0 - c) / 2.0;
            b1 = 1.0 - c;
            b2 = (1.0 - c) / 2.0;
            a0 = 1.0 + alpha;
            a1 = -2.0 * c;
            a2 = 1.0 - alpha;
        }
        BandKind::Peaking => {
            let a = 10f64.powf(gain_db / 40.0);
            b0 = 1.0 + alpha * a;
            b1 = -2.0 * c;
            b2 = 1.0 - alpha * a;
            a0 = 1.0 + alpha / a;
            a1 = -2.0 * c;
            a2 = 1.0 - alpha / a;
        }
        BandKind::LowShelf => {
            let a = 10f64.powf(gain_db / 40.0);
            let sq = 2.0 * a.sqrt() * alpha;
            b0 = a * ((a + 1.0) - (a - 1.0) * c + sq);
            b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * c);
            b2 = a * ((a + 1.0) - (a - 1.0) * c - sq);
            a0 = (a + 1.0) + (a - 1.0) * c + sq;
            a1 = -2.0 * ((a - 1.0) + (a + 1.0) * c);
            a2 = (a + 1.0) + (a - 1.0) * c - sq;
        }
        BandKind::HighShelf => {
            let a = 10f64.powf(gain_db / 40.0);
            let sq = 2.0 * a.sqrt() * alpha;
            b0 = a * ((a + 1.0) + (a - 1.0) * c + sq);
            b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * c);
            b2 = a * ((a + 1.0) + (a - 1.0) * c - sq);
            a0 = (a + 1.0) - (a - 1.0) * c + sq;
            a1 = 2.0 * ((a - 1.0) - (a + 1.0) * c);
            a2 = (a + 1.0) - (a - 1.0) * c - sq;
        }
    }
    // A peaking/shelf band at ±0 dB is a no-op; skip it.
    let active = match kind {
        BandKind::Peaking | BandKind::LowShelf | BandKind::HighShelf => gain_db.abs() > 0.05,
        _ => true,
    };
    b0 /= a0;
    b1 /= a0;
    b2 /= a0;
    a1 /= a0;
    a2 /= a0;
    Biquad {
        b0: b0 as f32,
        b1: b1 as f32,
        b2: b2 as f32,
        a1: a1 as f32,
        a2: a2 as f32,
        z1: 0.0,
        z2: 0.0,
        active,
    }
}

// ---------------------------------------------------------------------------
// EQ model (project) — 6 bands like a console channel strip
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BandKind {
    LowCut,
    LowShelf,
    Peaking,
    HighShelf,
    HighCut,
}

impl BandKind {
    pub fn label(&self) -> &'static str {
        match self {
            BandKind::LowCut => "Low cut",
            BandKind::LowShelf => "Low shelf",
            BandKind::Peaking => "Bell",
            BandKind::HighShelf => "High shelf",
            BandKind::HighCut => "High cut",
        }
    }

    /// Peaking/shelf bands expose gain; cut bands don't.
    pub fn has_gain(&self) -> bool {
        matches!(self, BandKind::Peaking | BandKind::LowShelf | BandKind::HighShelf)
    }
}

/// One EQ band. `freq` in Hz, `gain_db` for peaking/shelf, `q` resonance.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EqBand {
    pub enabled: bool,
    pub kind: BandKind,
    pub freq: f64,
    pub gain_db: f64,
    pub q: f64,
}

impl EqBand {
    pub fn is_neutral(&self) -> bool {
        match self.kind {
            BandKind::Peaking | BandKind::LowShelf | BandKind::HighShelf => {
                !self.enabled || self.gain_db.abs() < 0.05
            }
            // Cut bands at extreme frequencies are effectively transparent.
            BandKind::LowCut => !self.enabled || self.freq <= 15.0,
            BandKind::HighCut => !self.enabled || self.freq >= 22_000.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EqState {
    pub enabled: bool,
    pub bands: [EqBand; 6],
}

impl Default for EqState {
    fn default() -> Self {
        EqState {
            enabled: false,
            bands: default_eq_bands(),
        }
    }
}

pub fn default_eq_bands() -> [EqBand; 6] {
    [
        EqBand { enabled: false, kind: BandKind::LowCut, freq: 80.0, gain_db: 0.0, q: 0.71 },
        EqBand { enabled: false, kind: BandKind::LowShelf, freq: 120.0, gain_db: 0.0, q: 0.71 },
        EqBand { enabled: false, kind: BandKind::Peaking, freq: 500.0, gain_db: 0.0, q: 0.71 },
        EqBand { enabled: false, kind: BandKind::Peaking, freq: 2500.0, gain_db: 0.0, q: 0.71 },
        EqBand { enabled: false, kind: BandKind::HighShelf, freq: 8000.0, gain_db: 0.0, q: 0.71 },
        EqBand { enabled: false, kind: BandKind::HighCut, freq: 18_000.0, gain_db: 0.0, q: 0.71 },
    ]
}

/// Named EQ presets (same list the reference app shows in the EQ dialog).
pub fn eq_presets() -> &'static [(&'static str, EqState)] {
    &EQ_PRESETS
}

static EQ_PRESETS: std::sync::LazyLock<Vec<(&'static str, EqState)>> = std::sync::LazyLock::new(|| {
    vec![
    ("Default", EqState { enabled: true, bands: default_eq_bands() }),
    (
        "Flat",
        EqState {
            enabled: true,
            bands: [
                EqBand { enabled: false, kind: BandKind::LowCut, freq: 20.0, gain_db: 0.0, q: 0.71 },
                EqBand { enabled: false, kind: BandKind::LowShelf, freq: 120.0, gain_db: 0.0, q: 0.71 },
                EqBand { enabled: false, kind: BandKind::Peaking, freq: 500.0, gain_db: 0.0, q: 0.71 },
                EqBand { enabled: false, kind: BandKind::Peaking, freq: 2500.0, gain_db: 0.0, q: 0.71 },
                EqBand { enabled: false, kind: BandKind::HighShelf, freq: 8000.0, gain_db: 0.0, q: 0.71 },
                EqBand { enabled: false, kind: BandKind::HighCut, freq: 22_000.0, gain_db: 0.0, q: 0.71 },
            ],
        },
    ),
    (
        "Gentle glue",
        EqState {
            enabled: true,
            bands: [
                EqBand { enabled: true, kind: BandKind::LowCut, freq: 40.0, gain_db: 0.0, q: 0.71 },
                EqBand { enabled: true, kind: BandKind::LowShelf, freq: 120.0, gain_db: 1.0, q: 0.71 },
                EqBand { enabled: true, kind: BandKind::Peaking, freq: 350.0, gain_db: -1.0, q: 1.0 },
                EqBand { enabled: true, kind: BandKind::Peaking, freq: 3000.0, gain_db: 0.8, q: 0.9 },
                EqBand { enabled: true, kind: BandKind::HighShelf, freq: 9000.0, gain_db: 1.2, q: 0.71 },
                EqBand { enabled: false, kind: BandKind::HighCut, freq: 20_000.0, gain_db: 0.0, q: 0.71 },
            ],
        },
    ),
    (
        "Voice clarity",
        EqState {
            enabled: true,
            bands: [
                EqBand { enabled: true, kind: BandKind::LowCut, freq: 100.0, gain_db: 0.0, q: 0.71 },
                EqBand { enabled: true, kind: BandKind::LowShelf, freq: 150.0, gain_db: -2.0, q: 0.71 },
                EqBand { enabled: true, kind: BandKind::Peaking, freq: 300.0, gain_db: -2.5, q: 1.2 },
                EqBand { enabled: true, kind: BandKind::Peaking, freq: 2800.0, gain_db: 3.0, q: 0.9 },
                EqBand { enabled: true, kind: BandKind::HighShelf, freq: 8000.0, gain_db: 2.0, q: 0.71 },
                EqBand { enabled: false, kind: BandKind::HighCut, freq: 16_000.0, gain_db: 0.0, q: 0.71 },
            ],
        },
    ),
    (
        "De-esser",
        EqState {
            enabled: true,
            bands: [
                EqBand { enabled: false, kind: BandKind::LowCut, freq: 80.0, gain_db: 0.0, q: 0.71 },
                EqBand { enabled: false, kind: BandKind::LowShelf, freq: 120.0, gain_db: 0.0, q: 0.71 },
                EqBand { enabled: false, kind: BandKind::Peaking, freq: 500.0, gain_db: 0.0, q: 0.71 },
                EqBand { enabled: true, kind: BandKind::Peaking, freq: 6500.0, gain_db: -3.5, q: 2.2 },
                EqBand { enabled: true, kind: BandKind::Peaking, freq: 9500.0, gain_db: -2.5, q: 2.5 },
                EqBand { enabled: false, kind: BandKind::HighCut, freq: 20_000.0, gain_db: 0.0, q: 0.71 },
            ],
        },
    ),
    (
        "Tame the low end",
        EqState {
            enabled: true,
            bands: [
                EqBand { enabled: true, kind: BandKind::LowCut, freq: 60.0, gain_db: 0.0, q: 0.71 },
                EqBand { enabled: true, kind: BandKind::LowShelf, freq: 140.0, gain_db: -3.0, q: 0.71 },
                EqBand { enabled: true, kind: BandKind::Peaking, freq: 250.0, gain_db: -2.0, q: 1.1 },
                EqBand { enabled: false, kind: BandKind::Peaking, freq: 2500.0, gain_db: 0.0, q: 0.71 },
                EqBand { enabled: false, kind: BandKind::HighShelf, freq: 8000.0, gain_db: 0.0, q: 0.71 },
                EqBand { enabled: false, kind: BandKind::HighCut, freq: 20_000.0, gain_db: 0.0, q: 0.71 },
            ],
        },
    ),
    (
        "Broadcast loud",
        EqState {
            enabled: true,
            bands: [
                EqBand { enabled: true, kind: BandKind::LowCut, freq: 90.0, gain_db: 0.0, q: 0.71 },
                EqBand { enabled: true, kind: BandKind::LowShelf, freq: 180.0, gain_db: 2.0, q: 0.71 },
                EqBand { enabled: true, kind: BandKind::Peaking, freq: 900.0, gain_db: -1.5, q: 1.0 },
                EqBand { enabled: true, kind: BandKind::Peaking, freq: 3400.0, gain_db: 2.8, q: 0.8 },
                EqBand { enabled: true, kind: BandKind::HighShelf, freq: 7500.0, gain_db: 2.5, q: 0.71 },
                EqBand { enabled: true, kind: BandKind::HighCut, freq: 15_000.0, gain_db: 0.0, q: 0.71 },
            ],
        },
    ),
    (
        "Music mastering",
        EqState {
            enabled: true,
            bands: [
                EqBand { enabled: true, kind: BandKind::LowCut, freq: 25.0, gain_db: 0.0, q: 0.71 },
                EqBand { enabled: true, kind: BandKind::LowShelf, freq: 100.0, gain_db: 1.5, q: 0.71 },
                EqBand { enabled: true, kind: BandKind::Peaking, freq: 450.0, gain_db: -0.8, q: 1.0 },
                EqBand { enabled: true, kind: BandKind::Peaking, freq: 2200.0, gain_db: 1.0, q: 0.8 },
                EqBand { enabled: true, kind: BandKind::HighShelf, freq: 10_000.0, gain_db: 1.8, q: 0.71 },
                EqBand { enabled: false, kind: BandKind::HighCut, freq: 22_000.0, gain_db: 0.0, q: 0.71 },
            ],
        },
    ),
]
});

impl EqState {
    pub fn is_neutral(&self) -> bool {
        !self.enabled || self.bands.iter().all(|b| b.is_neutral())
    }

    /// Total magnitude response in dB at `freq` (for the editor curve).
    pub fn response_db(&self, freq: f64, sample_rate: f64) -> f64 {
        if !self.enabled {
            return 0.0;
        }
        self.bands
            .iter()
            .filter(|b| b.enabled)
            .map(|b| design(b.kind, b.freq, b.gain_db, b.q, sample_rate).magnitude_db(freq, sample_rate))
            .sum()
    }
}

// ---------------------------------------------------------------------------
// Compressor model (project) — 3-band, like the reference multiband unit
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CompBand {
    pub enabled: bool,
    pub threshold_db: f64,
    /// 1.0 = no compression, up to 20.
    pub ratio: f64,
    pub attack_ms: f64,
    pub release_ms: f64,
    /// Output trim for this band (dB).
    pub gain_db: f64,
}

impl Default for CompBand {
    fn default() -> Self {
        CompBand { enabled: false, threshold_db: -24.0, ratio: 3.0, attack_ms: 10.0, release_ms: 150.0, gain_db: 0.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompState {
    pub enabled: bool,
    pub low: CompBand,
    pub mid: CompBand,
    pub high: CompBand,
    /// Crossover frequencies (Low | Mid | High).
    pub xover_low_hz: f64,
    pub xover_high_hz: f64,
}

impl Default for CompState {
    fn default() -> Self {
        CompState {
            enabled: false,
            low: CompBand { enabled: true, threshold_db: -26.0, ratio: 2.5, attack_ms: 12.0, release_ms: 150.0, gain_db: 0.0 },
            mid: CompBand { enabled: true, threshold_db: -24.0, ratio: 2.0, attack_ms: 8.0, release_ms: 120.0, gain_db: 0.0 },
            high: CompBand { enabled: true, threshold_db: -22.0, ratio: 2.0, attack_ms: 4.0, release_ms: 90.0, gain_db: 0.0 },
            xover_low_hz: 200.0,
            xover_high_hz: 3000.0,
        }
    }
}

/// Named multiband compressor presets (reference unit's preset list).
pub fn comp_presets() -> &'static [(&'static str, CompState)] {
    &COMP_PRESETS
}

static COMP_PRESETS: std::sync::LazyLock<Vec<(&'static str, CompState)>> = std::sync::LazyLock::new(|| {
    vec![
    ("Default", CompState::default()),
    (
        "Gentle glue",
        CompState {
            enabled: true,
            low: CompBand { enabled: true, threshold_db: -22.0, ratio: 1.6, attack_ms: 20.0, release_ms: 220.0, gain_db: 1.0 },
            mid: CompBand { enabled: true, threshold_db: -20.0, ratio: 1.5, attack_ms: 15.0, release_ms: 180.0, gain_db: 1.0 },
            high: CompBand { enabled: true, threshold_db: -18.0, ratio: 1.5, attack_ms: 8.0, release_ms: 140.0, gain_db: 0.5 },
            xover_low_hz: 180.0,
            xover_high_hz: 2800.0,
        },
    ),
    (
        "Voice clarity",
        CompState {
            enabled: true,
            low: CompBand { enabled: true, threshold_db: -30.0, ratio: 4.0, attack_ms: 10.0, release_ms: 120.0, gain_db: -2.0 },
            mid: CompBand { enabled: true, threshold_db: -22.0, ratio: 3.0, attack_ms: 6.0, release_ms: 100.0, gain_db: 2.0 },
            high: CompBand { enabled: true, threshold_db: -20.0, ratio: 2.0, attack_ms: 3.0, release_ms: 80.0, gain_db: 1.0 },
            xover_low_hz: 160.0,
            xover_high_hz: 2500.0,
        },
    ),
    (
        "De-esser",
        CompState {
            enabled: true,
            low: CompBand { enabled: false, threshold_db: -30.0, ratio: 2.0, attack_ms: 12.0, release_ms: 150.0, gain_db: 0.0 },
            mid: CompBand { enabled: false, threshold_db: -24.0, ratio: 2.0, attack_ms: 8.0, release_ms: 120.0, gain_db: 0.0 },
            high: CompBand { enabled: true, threshold_db: -26.0, ratio: 6.0, attack_ms: 1.0, release_ms: 60.0, gain_db: 0.0 },
            xover_low_hz: 200.0,
            xover_high_hz: 5000.0,
        },
    ),
    (
        "Tame the low end",
        CompState {
            enabled: true,
            low: CompBand { enabled: true, threshold_db: -24.0, ratio: 5.0, attack_ms: 15.0, release_ms: 180.0, gain_db: -1.0 },
            mid: CompBand { enabled: false, threshold_db: -20.0, ratio: 2.0, attack_ms: 10.0, release_ms: 120.0, gain_db: 0.0 },
            high: CompBand { enabled: false, threshold_db: -18.0, ratio: 2.0, attack_ms: 5.0, release_ms: 90.0, gain_db: 0.0 },
            xover_low_hz: 140.0,
            xover_high_hz: 3000.0,
        },
    ),
    (
        "Broadcast loud",
        CompState {
            enabled: true,
            low: CompBand { enabled: true, threshold_db: -26.0, ratio: 3.5, attack_ms: 8.0, release_ms: 120.0, gain_db: 2.0 },
            mid: CompBand { enabled: true, threshold_db: -24.0, ratio: 3.5, attack_ms: 5.0, release_ms: 100.0, gain_db: 2.0 },
            high: CompBand { enabled: true, threshold_db: -22.0, ratio: 3.0, attack_ms: 2.0, release_ms: 70.0, gain_db: 1.5 },
            xover_low_hz: 180.0,
            xover_high_hz: 3200.0,
        },
    ),
    (
        "Music mastering",
        CompState {
            enabled: true,
            low: CompBand { enabled: true, threshold_db: -20.0, ratio: 2.0, attack_ms: 25.0, release_ms: 260.0, gain_db: 1.0 },
            mid: CompBand { enabled: true, threshold_db: -18.0, ratio: 1.8, attack_ms: 18.0, release_ms: 200.0, gain_db: 1.0 },
            high: CompBand { enabled: true, threshold_db: -16.0, ratio: 1.6, attack_ms: 10.0, release_ms: 150.0, gain_db: 0.5 },
            xover_low_hz: 220.0,
            xover_high_hz: 3500.0,
        },
    ),
]
});

impl CompState {
    pub fn is_neutral(&self) -> bool {
        !self.enabled
    }
}

// ---------------------------------------------------------------------------
// Runtime processors (audio thread)
// ---------------------------------------------------------------------------

/// Per-channel runtime EQ. Recomputes coefficients lazily when the config
/// version changes.
pub struct EqRuntime {
    sample_rate: f64,
    version: u64,
    sections: [Biquad; 6],
}

impl EqRuntime {
    pub fn new(sample_rate: f64) -> Self {
        EqRuntime { sample_rate, version: 0, sections: [Biquad::bypass(); 6] }
    }

    pub fn reset(&mut self) {
        for s in &mut self.sections {
            s.reset();
        }
    }

    /// `state_version` must increment whenever the EqState may have changed.
    pub fn process(&mut self, x: f32, state: &EqState, state_version: u64) -> f32 {
        if state_version != self.version {
            for (i, b) in state.bands.iter().enumerate() {
                self.sections[i] = if state.enabled && b.enabled {
                    design(b.kind, b.freq, b.gain_db, b.q, self.sample_rate)
                } else {
                    Biquad::bypass()
                };
            }
            self.version = state_version;
        }
        if !state.enabled {
            return x;
        }
        let mut y = x;
        for s in &mut self.sections {
            y = s.process(y);
        }
        y
    }
}

/// Per-channel 3-band compressor. Bands split by two cascaded crossover
/// biquads (2nd-order Butterworth low/high pairs, phase-flat enough for the
/// purpose), each band runs its own envelope follower + soft-knee gain computer.
pub struct CompRuntime {
    sample_rate: f64,
    version: u64,
    // Crossovers (two channels worth is handled by two CompRuntime instances).
    low_pass1: Biquad,
    low_pass2: Biquad,
    high_pass1: Biquad,
    high_pass2: Biquad,
    // Envelope state per band.
    env_low: f32,
    env_mid: f32,
    env_high: f32,
    /// Last applied gain reduction per band (dB, ≤ 0) — for the UI meter.
    pub gr_low_db: f32,
    pub gr_mid_db: f32,
    pub gr_high_db: f32,
}

impl CompRuntime {
    pub fn new(sample_rate: f64) -> Self {
        CompRuntime {
            sample_rate,
            version: 0,
            low_pass1: Biquad::bypass(),
            low_pass2: Biquad::bypass(),
            high_pass1: Biquad::bypass(),
            high_pass2: Biquad::bypass(),
            env_low: 0.0,
            env_mid: 0.0,
            env_high: 0.0,
            gr_low_db: 0.0,
            gr_mid_db: 0.0,
            gr_high_db: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.env_low = 0.0;
        self.env_mid = 0.0;
        self.env_high = 0.0;
    }

    fn crossovers(&mut self, low_hz: f64, high_hz: f64) {
        let f_low = low_hz.clamp(40.0, 1000.0);
        let f_high = high_hz.clamp(1000.0, 12_000.0);
        // Butterworth Q = 0.7071, cascade two sections per split.
        self.low_pass1 = design(BandKind::HighCut, f_low, 0.0, 0.54, self.sample_rate);
        self.low_pass2 = design(BandKind::HighCut, f_low, 0.0, 1.31, self.sample_rate);
        self.high_pass1 = design(BandKind::LowCut, f_high, 0.0, 0.54, self.sample_rate);
        self.high_pass2 = design(BandKind::LowCut, f_high, 0.0, 1.31, self.sample_rate);
    }

    /// Process one sample. `state_version` changes when CompState changes.
    pub fn process(&mut self, x: f32, state: &CompState, state_version: u64) -> f32 {
        if !state.enabled {
            self.gr_low_db = 0.0;
            self.gr_mid_db = 0.0;
            self.gr_high_db = 0.0;
            return x;
        }
        if state_version != self.version {
            self.crossovers(state.xover_low_hz, state.xover_high_hz);
            self.version = state_version;
        }

        // Split: low = LP(x), high = HP(x), mid = x - low - high.
        let l1 = self.low_pass1.process(x);
        let low = self.low_pass2.process(l1);
        let h1 = self.high_pass1.process(x);
        let high = self.high_pass2.process(h1);
        let mid = x - low - high;

        let low = band(low, &state.low, &mut self.env_low, &mut self.gr_low_db, self.sample_rate);
        let mid = band(mid, &state.mid, &mut self.env_mid, &mut self.gr_mid_db, self.sample_rate);
        let high = band(high, &state.high, &mut self.env_high, &mut self.gr_high_db, self.sample_rate);

        low + mid + high
    }
}

fn band(x: f32, cfg: &CompBand, env: &mut f32, gr_db: &mut f32, sample_rate: f64) -> f32 {
    if !cfg.enabled {
        *gr_db = 0.0;
        return x;
    }
    // Envelope follower (peak, one-pole attack/release).
    let rect = x.abs();
    let fs = sample_rate;
    let atk = (-1.0 / (cfg.attack_ms.max(0.1) * 0.001 * fs)).exp() as f32;
    let rel = (-1.0 / (cfg.release_ms.max(5.0) * 0.001 * fs)).exp() as f32;
    if rect > *env {
        *env = atk * *env + (1.0 - atk) * rect;
    } else {
        *env = rel * *env + (1.0 - rel) * rect;
    }

    // Soft-knee gain computer (dB domain, knee = 6 dB).
    let env_db = 20.0 * ((*env).max(1e-6) as f64).log10();
    let th = cfg.threshold_db;
    let knee = 6.0f64;
    let over = if env_db > th + knee / 2.0 {
        env_db - th
    } else if env_db > th - knee / 2.0 {
        // quadratic knee
        let t = env_db - th + knee / 2.0;
        t * t / (2.0 * knee)
    } else {
        0.0
    };
    let target_gr = -(over * (1.0 - 1.0 / cfg.ratio.max(1.0))) as f32;
    // Smooth gain reduction movement (avoid modulating artifacts).
    *gr_db = 0.6 * *gr_db + 0.4 * target_gr;
    let gr_lin = 10f32.powf(*gr_db / 20.0);
    // No automatic makeup — the band `gain_db` knob is the user's makeup.
    x * gr_lin * 10f32.powf(cfg.gain_db as f32 / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bypass_is_transparent() {
        let mut b = Biquad::bypass();
        assert_eq!(b.process(0.37), 0.37);
        assert!((b.magnitude_db(1000.0, 48_000.0)).abs() < 1e-9);
    }

    #[test]
    fn peaking_boost_boosts() {
        let b = design(BandKind::Peaking, 1000.0, 12.0, 0.71, 48_000.0);
        assert!(b.active);
        let m = b.magnitude_db(1000.0, 48_000.0);
        assert!((m - 12.0).abs() < 0.6, "expected ~12 dB at center, got {m}");
    }

    #[test]
    fn lowcut_attacks_bass() {
        let b = design(BandKind::LowCut, 200.0, 0.0, 0.71, 48_000.0);
        assert!(b.magnitude_db(30.0, 48_000.0) < -20.0);
        assert!(b.magnitude_db(2000.0, 48_000.0).abs() < 1.0);
    }

    #[test]
    fn eq_response_sums_bands() {
        let mut st = EqState::default();
        st.enabled = true;
        st.bands[2].enabled = true;
        st.bands[2].gain_db = 6.0;
        st.bands[2].freq = 1000.0;
        let r = st.response_db(1000.0, 48_000.0);
        assert!((r - 6.0).abs() < 1.0, "response {r}");
        assert!(st.response_db(10_000.0, 48_000.0).abs() < 0.5);
    }

    #[test]
    fn compressor_reduces_loud() {
        let mut st = CompState::default();
        st.enabled = true;
        let mut rt = CompRuntime::new(48_000.0);
        // Feed a 1 kHz sine at 0 dBFS-ish; expect steady-state gain reduction.
        let mut last_gr = 0.0f32;
        for i in 0..48_000 {
            let s = (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / 48_000.0).sin() as f32 * 0.9;
            let _ = rt.process(s, &st, 1);
            last_gr = rt.gr_mid_db;
        }
        assert!(last_gr < -3.0, "expected gain reduction, got {last_gr}");
        // Quiet signal should not be compressed.
        let mut rt2 = CompRuntime::new(48_000.0);
        let mut gr2 = 0.0f32;
        for i in 0..48_000 {
            let s = (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / 48_000.0).sin() as f32 * 0.01;
            let _ = rt2.process(s, &st, 1);
            gr2 = rt2.gr_mid_db;
        }
        assert!(gr2 > -0.5, "quiet signal compressed: {gr2}");
    }

    #[test]
    fn comp_split_conserves_energy() {
        // Sum of the three bands must reconstruct the input (linear split).
        let mut st = CompState::default();
        st.enabled = true;
        st.low.enabled = false;
        st.mid.enabled = false;
        st.high.enabled = false;
        let mut rt = CompRuntime::new(48_000.0);
        let mut max_err = 0.0f32;
        for i in 0..4800 {
            let s = (2.0 * std::f64::consts::PI * 300.0 * i as f64 / 48_000.0).sin() as f32;
            let y = rt.process(s, &st, 1);
            max_err = max_err.max((s - y).abs());
        }
        assert!(max_err < 0.05, "split reconstruction error {max_err}");
    }
}
