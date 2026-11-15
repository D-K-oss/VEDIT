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

    /// Load normalized coefficients (a0 assumed 1). Used by the loudness
    /// meter's fixed ITU-R BS.1770 stages.
    pub fn load(&mut self, b0: f64, b1: f64, b2: f64, a1: f64, a2: f64) {
        self.b0 = b0 as f32;
        self.b1 = b1 as f32;
        self.b2 = b2 as f32;
        self.a1 = a1 as f32;
        self.a2 = a2 as f32;
        self.z1 = 0.0;
        self.z2 = 0.0;
        self.active = true;
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

// ---------------------------------------------------------------------------
// v0.5.0 Fairlight — notch biquad, FFT analyzer, dynamics processor
// (expander/gate → compressor → limiter), track FX inserts and an
// ITU-R BS.1770-4 loudness meter.
// ---------------------------------------------------------------------------

/// Notch biquad (0 dB everywhere, deep cut at `freq`). Used by De-Hummer.
pub fn biquad_notch(freq: f64, q: f64, sample_rate: f64) -> Biquad {
    let fs = sample_rate.max(8_000.0);
    let f0 = freq.clamp(10.0, fs * 0.49);
    let q = q.max(0.05);
    let w0 = 2.0 * std::f64::consts::PI * f0 / fs;
    let (s, c) = w0.sin_cos();
    let alpha = s / (2.0 * q);
    let a0 = 1.0 + alpha;
    let b0 = (1.0 / a0) as f32;
    let b1 = (-2.0 * c / a0) as f32;
    let b2 = (1.0 / a0) as f32;
    let a1 = (-2.0 * c / a0) as f32;
    let a2 = ((1.0 - alpha) / a0) as f32;
    Biquad { b0, b1, b2, a1, a2, z1: 0.0, z2: 0.0, active: true }
}

/// Low-shelf helper (De-Basser).
pub fn biquad_low_shelf(freq: f64, gain_db: f64, q: f64, sample_rate: f64) -> Biquad {
    design(BandKind::LowShelf, freq, gain_db, q, sample_rate)
}

// --- FFT (real-time spectrum analyzer for the Fairlight EQ) ----------------

/// In-place iterative radix-2 FFT over `re`/`im` (length must be a power of
/// two). Small and allocation-free — good enough for a 2048-point UI
/// analyzer at ~20 fps.
fn fft_inplace(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    if n <= 1 || !n.is_power_of_two() {
        return;
    }
    // Bit-reversal permutation.
    let bits = n.trailing_zeros();
    for i in 0..n {
        let rev = ((i as u32).reverse_bits() >> (32 - bits)) as usize;
        if rev > i {
            re.swap(i, rev);
            im.swap(i, rev);
        }
    }
    // Butterflies.
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        let mut start = 0;
        while start < n {
            let mut cwr = 1.0f32;
            let mut cwi = 0.0f32;
            for k in 0..len / 2 {
                let i0 = start + k;
                let i1 = i0 + len / 2;
                let (xr, xi) = (re[i1] * cwr - im[i1] * cwi, re[i1] * cwi + im[i1] * cwr);
                re[i1] = re[i0] - xr;
                im[i1] = im[i0] - xi;
                re[i0] += xr;
                im[i0] += xi;
                let nwr = cwr * wr - cwi * wi;
                cwi = cwr * wi + cwi * wr;
                cwr = nwr;
            }
            start += len;
        }
        len <<= 1;
    }
}

/// FFT size used by [`fft_spectrum_db`].
pub const FFT_SIZE: usize = 2048;

/// Magnitude spectrum in dB (0 = full scale, −100 = floor) for `samples`
/// (any length; wrapped into one 2048-point Hann window). Output has
/// `FFT_SIZE / 2` bins; bin i is centered at `i * sample_rate / FFT_SIZE`.
pub fn fft_spectrum_db(samples: &[f32], out: &mut Vec<f32>) {
    out.clear();
    let n = FFT_SIZE;
    let mut re = vec![0.0f32; n];
    let mut im = vec![0.0f32; n];
    if samples.is_empty() {
        out.resize(n / 2, -100.0);
        return;
    }
    // Take the LAST n samples (most recent audio), Hann window.
    let take = samples.len().min(n);
    let start = samples.len() - take;
    for (i, s) in samples[start..].iter().enumerate() {
        let w = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / take as f32).cos();
        re[i] = *s * w;
    }
    fft_inplace(&mut re, &mut im);
    out.reserve(n / 2);
    for i in 0..n / 2 {
        let mag = (re[i] * re[i] + im[i] * im[i]).sqrt() / (take as f32 * 0.5);
        let db = 20.0 * mag.max(1e-5).log10();
        out.push(db.clamp(-100.0, 6.0));
    }
}

// --- Dynamics: Expander/Gate → Compressor → Limiter ------------------------

fn ms_to_coef(ms: f64, sample_rate: f64) -> f64 {
    // One-pole coefficient reaching ~63 % in `ms`.
    let t = (ms.max(0.01) / 1000.0) * sample_rate;
    (-1.0 / t.max(1.0)).exp()
}

/// Expander / gate section ("Espansore / Gate").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateState {
    pub enabled: bool,
    /// Open threshold in dBFS ("Soglia").
    pub threshold_db: f64,
    /// Maximum attenuation when closed, in dB ("Intervallo").
    pub range_db: f64,
    /// Expansion ratio below the threshold, written 1:1.1 → stored 1.1
    /// ("Rapporto").
    pub ratio: f64,
    pub attack_ms: f64,
    /// Hold time before the release ramp starts ("Tenuta").
    pub hold_ms: f64,
    pub release_ms: f64,
}

impl Default for GateState {
    fn default() -> Self {
        GateState {
            enabled: false,
            threshold_db: -35.0,
            range_db: 18.0,
            ratio: 1.1,
            attack_ms: 1.0,
            hold_ms: 20.0,
            release_ms: 120.0,
        }
    }
}

/// Compressor section of the classic dynamics processor ("Compressore").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DynCompState {
    pub enabled: bool,
    pub threshold_db: f64,
    /// e.g. 2.0 → 2.0:1 ("Rapporto").
    pub ratio: f64,
    /// Soft-knee width in dB ("Curva").
    pub knee_db: f64,
    /// Dry/wet blend 0..1 ("Mix").
    pub mix: f64,
    pub attack_ms: f64,
    pub hold_ms: f64,
    pub release_ms: f64,
    /// Makeup gain in dB ("Recupero").
    pub makeup_db: f64,
}

impl Default for DynCompState {
    fn default() -> Self {
        DynCompState {
            enabled: false,
            threshold_db: -15.0,
            ratio: 2.0,
            knee_db: 6.0,
            mix: 0.0,
            attack_ms: 5.0,
            hold_ms: 5.0,
            release_ms: 120.0,
            makeup_db: 0.0,
        }
    }
}

/// Limiter section ("Limitatore").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimiterState {
    pub enabled: bool,
    /// Ceiling in dBFS ("Soglia").
    pub threshold_db: f64,
    pub release_ms: f64,
}

impl Default for LimiterState {
    fn default() -> Self {
        LimiterState { enabled: false, threshold_db: -1.0, release_ms: 60.0 }
    }
}

/// The full three-module dynamics processor serialized on the track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DynamicsState {
    /// Master enable for the whole processor.
    pub enabled: bool,
    #[serde(default)]
    pub gate: GateState,
    #[serde(default)]
    pub comp: DynCompState,
    #[serde(default)]
    pub limiter: LimiterState,
}

impl DynamicsState {
    pub fn is_neutral(&self) -> bool {
        !self.enabled
    }
}

/// Static input → output transfer (dB domain) for the UI graph.
pub fn dynamics_transfer_db(st: &DynamicsState, in_db: f64) -> f64 {
    let mut out = in_db;
    if st.gate.enabled {
        let over = (st.gate.threshold_db - out).max(0.0);
        out -= (over * (st.gate.ratio.max(1.0) - 1.0)).min(st.gate.range_db.max(0.0));
    }
    if st.comp.enabled {
        let th = st.comp.threshold_db;
        let knee = st.comp.knee_db.max(0.0);
        let over = if out > th + knee / 2.0 {
            out - th
        } else if out > th - knee / 2.0 {
            let t = out - th + knee / 2.0;
            t * t / (2.0 * knee)
        } else {
            0.0
        };
        out -= over * (1.0 - 1.0 / st.comp.ratio.max(1.0));
        out += st.comp.makeup_db;
    }
    if st.limiter.enabled {
        out = out.min(st.limiter.threshold_db);
    }
    out
}

/// Gain reduction of each stage in dB (≤ 0), for metering.
#[derive(Debug, Clone, Copy, Default)]
pub struct DynGr {
    pub gate_db: f32,
    pub comp_db: f32,
    pub limiter_db: f32,
}

impl DynGr {
    pub fn max_db(&self) -> f32 {
        self.gate_db.min(self.comp_db).min(self.limiter_db)
    }
}

/// Runtime state of the dynamics processor (one per channel).
pub struct DynamicsRuntime {
    sample_rate: f64,
    version: u64,
    cfg: DynamicsState,
    // gate
    g_env: f32,
    g_gain: f32,
    g_hold_left: usize,
    // comp
    c_env: f32,
    c_gr: f32,
    c_hold_left: usize,
    // limiter
    l_env: f32,
    l_gr: f32,
    pub gr: DynGr,
}

impl DynamicsRuntime {
    pub fn new(sample_rate: f64) -> Self {
        DynamicsRuntime {
            sample_rate,
            version: 0,
            cfg: DynamicsState::default(),
            g_env: 0.0,
            g_gain: 1.0,
            g_hold_left: 0,
            c_env: 0.0,
            c_gr: 0.0,
            c_hold_left: 0,
            l_env: 0.0,
            l_gr: 0.0,
            gr: DynGr::default(),
        }
    }

    pub fn reset(&mut self) {
        self.g_env = 0.0;
        self.g_gain = 1.0;
        self.c_env = 0.0;
        self.c_gr = 0.0;
        self.l_env = 0.0;
        self.l_gr = 0.0;
        self.gr = DynGr::default();
    }

    pub fn process(&mut self, x: f32, state: &DynamicsState, version: u64) -> f32 {
        if !state.enabled {
            self.gr = DynGr::default();
            return x;
        }
        if self.version != version {
            self.cfg = state.clone();
            self.version = version;
        }
        let sr = self.sample_rate;
        let mut y = x;

        // ---- gate / expander ----
        let g = &self.cfg.gate;
        let mut gate_gr = 0.0f32;
        if g.enabled {
            let rect = y.abs();
            // Peak follower for the detector.
            if rect > self.g_env {
                let a = ms_to_coef(g.attack_ms, sr) as f32;
                self.g_env = a * self.g_env + (1.0 - a) * rect;
                self.g_hold_left = (g.hold_ms / 1000.0 * sr) as usize;
            } else if self.g_hold_left > 0 {
                self.g_hold_left -= 1;
            } else {
                let r = ms_to_coef(g.release_ms, sr) as f32;
                self.g_env = r * self.g_env + (1.0 - r) * rect;
            }
            let env_db = 20.0 * (self.g_env.max(1e-6) as f64).log10();
            let over = (g.threshold_db - env_db).max(0.0);
            let atten = (over * (g.ratio.max(1.0) - 1.0)).min(g.range_db.max(0.0));
            let target = 10f64.powf(-atten / 20.0) as f32;
            // Open fast (attack), close with release.
            let coef = if target > self.g_gain {
                ms_to_coef(g.attack_ms.max(0.05), sr) as f32
            } else {
                ms_to_coef(g.release_ms.max(0.05), sr) as f32
            };
            self.g_gain = coef * self.g_gain + (1.0 - coef) * target;
            gate_gr = (1.0 - self.g_gain).min(1.0) * -(g.range_db.max(0.0) as f32);
            if gate_gr > -0.05 {
                gate_gr = 0.0;
            }
            y *= self.g_gain;
        }

        // ---- compressor ----
        let c = &self.cfg.comp;
        let mut comp_gr = 0.0f32;
        if c.enabled {
            let rect = y.abs();
            let atk = ms_to_coef(c.attack_ms, sr) as f32;
            let rel = ms_to_coef(c.release_ms, sr) as f32;
            if rect > self.c_env {
                self.c_env = atk * self.c_env + (1.0 - atk) * rect;
                self.c_hold_left = (c.hold_ms / 1000.0 * sr) as usize;
            } else if self.c_hold_left > 0 {
                self.c_hold_left -= 1;
            } else {
                self.c_env = rel * self.c_env + (1.0 - rel) * rect;
            }
            let env_db = 20.0 * (self.c_env.max(1e-6) as f64).log10();
            let th = c.threshold_db;
            let knee = c.knee_db.max(0.0);
            let over = if env_db > th + knee / 2.0 {
                env_db - th
            } else if env_db > th - knee / 2.0 {
                let t = env_db - th + knee / 2.0;
                t * t / (2.0 * knee)
            } else {
                0.0
            };
            let target_gr = -(over * (1.0 - 1.0 / c.ratio.max(1.0))) as f32;
            self.c_gr = 0.5 * self.c_gr + 0.5 * target_gr;
            comp_gr = self.c_gr;
            let wet = y * 10f32.powf(self.c_gr / 20.0) * 10f32.powf(c.makeup_db as f32 / 20.0);
            let mix = c.mix.clamp(0.0, 1.0) as f32;
            y = wet * mix + y * (1.0 - mix);
            if mix <= 0.0 {
                y = y * 10f32.powf(c.makeup_db as f32 / 20.0);
            }
        }

        // ---- limiter ----
        let l = &self.cfg.limiter;
        let mut lim_gr = 0.0f32;
        if l.enabled {
            let rect = y.abs();
            // Fast attack (0.1 ms), parametric release.
            if rect > self.l_env {
                let a = ms_to_coef(0.1, sr) as f32;
                self.l_env = a * self.l_env + (1.0 - a) * rect;
            } else {
                let r = ms_to_coef(l.release_ms, sr) as f32;
                self.l_env = r * self.l_env + (1.0 - r) * rect;
            }
            let env_db = 20.0 * (self.l_env.max(1e-6) as f64).log10();
            let over = (env_db - l.threshold_db).max(0.0);
            let target_gr = -over as f32;
            self.l_gr = 0.7 * self.l_gr + 0.3 * target_gr;
            lim_gr = self.l_gr;
            if lim_gr > -0.05 {
                lim_gr = 0.0;
            }
            y *= 10f32.powf(self.l_gr / 20.0);
        }

        self.gr = DynGr { gate_db: gate_gr, comp_db: comp_gr, limiter_db: lim_gr };
        y
    }
}

// --- Track FX inserts -------------------------------------------------------

/// Feedback delay ("Delay" — short, "Echo" — long taps share the type).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DelayParams {
    pub enabled: bool,
    /// Delay time in milliseconds (0..2000).
    pub time_ms: f64,
    /// Feedback 0..0.95.
    pub feedback: f64,
    /// Dry/wet 0..1.
    pub mix: f64,
}

impl DelayParams {
    pub fn echo() -> Self {
        DelayParams { enabled: true, time_ms: 320.0, feedback: 0.45, mix: 0.35 }
    }
    pub fn delay() -> Self {
        DelayParams { enabled: true, time_ms: 120.0, feedback: 0.3, mix: 0.25 }
    }
}

/// Modulated delay ("Chorus").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChorusParams {
    pub enabled: bool,
    /// LFO rate in Hz (0.05..10).
    pub rate_hz: f64,
    /// Modulation depth in ms (0..15).
    pub depth_ms: f64,
    pub mix: f64,
}

impl Default for ChorusParams {
    fn default() -> Self {
        ChorusParams { enabled: true, rate_hz: 0.8, depth_ms: 3.0, mix: 0.4 }
    }
}

/// Short modulated delay with feedback ("Flanger").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlangerParams {
    pub enabled: bool,
    pub rate_hz: f64,
    pub depth_ms: f64,
    /// Regeneration 0..0.95.
    pub feedback: f64,
    pub mix: f64,
}

impl Default for FlangerParams {
    fn default() -> Self {
        FlangerParams { enabled: true, rate_hz: 0.3, depth_ms: 1.5, feedback: 0.5, mix: 0.5 }
    }
}

/// Wave-shaper ("Distortion").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistortionParams {
    pub enabled: bool,
    /// Drive amount 1..20.
    pub drive: f64,
    pub mix: f64,
}

impl Default for DistortionParams {
    fn default() -> Self {
        DistortionParams { enabled: true, drive: 4.0, mix: 0.6 }
    }
}

/// Four notch filters at base·n ("De-Hummer").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeHummerParams {
    pub enabled: bool,
    /// Mains base frequency: 50 or 60 Hz.
    pub base_hz: f64,
    /// Notch depth in dB (fixed shape, 6..40).
    pub depth_db: f64,
}

impl Default for DeHummerParams {
    fn default() -> Self {
        DeHummerParams { enabled: true, base_hz: 50.0, depth_db: 18.0 }
    }
}

/// Low-shelf cut ("De-Basser").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeBasserParams {
    pub enabled: bool,
    pub freq_hz: f64,
    pub gain_db: f64,
}

impl Default for DeBasserParams {
    fn default() -> Self {
        DeBasserParams { enabled: true, freq_hz: 110.0, gain_db: -6.0 }
    }
}

/// The per-track FX insert rack — `None`/disabled slots bypass. Serialized
/// on the track (all back-compatible defaults).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TrackFx {
    #[serde(default)]
    pub delay: Option<DelayParams>,
    #[serde(default)]
    pub echo: Option<DelayParams>,
    #[serde(default)]
    pub chorus: Option<ChorusParams>,
    #[serde(default)]
    pub flanger: Option<FlangerParams>,
    #[serde(default)]
    pub distortion: Option<DistortionParams>,
    #[serde(default)]
    pub de_hummer: Option<DeHummerParams>,
    #[serde(default)]
    pub de_basser: Option<DeBasserParams>,
}

impl TrackFx {
    pub fn is_empty(&self) -> bool {
        self.delay.as_ref().map_or(true, |f| !f.enabled)
            && self.echo.as_ref().map_or(true, |f| !f.enabled)
            && self.chorus.as_ref().map_or(true, |f| !f.enabled)
            && self.flanger.as_ref().map_or(true, |f| !f.enabled)
            && self.distortion.as_ref().map_or(true, |f| !f.enabled)
            && self.de_hummer.as_ref().map_or(true, |f| !f.enabled)
            && self.de_basser.as_ref().map_or(true, |f| !f.enabled)
    }

    /// Number of ACTIVE inserts (for the "FX 3" badge).
    pub fn active_count(&self) -> usize {
        [
            self.delay.as_ref().map_or(false, |f| f.enabled),
            self.echo.as_ref().map_or(false, |f| f.enabled),
            self.chorus.as_ref().map_or(false, |f| f.enabled),
            self.flanger.as_ref().map_or(false, |f| f.enabled),
            self.distortion.as_ref().map_or(false, |f| f.enabled),
            self.de_hummer.as_ref().map_or(false, |f| f.enabled),
            self.de_basser.as_ref().map_or(false, |f| f.enabled),
        ]
        .iter()
        .filter(|b| **b)
        .count()
    }
}

const FX_MAX_DELAY: usize = 96000; // 2 s at 48 kHz

/// Runtime of the FX rack (one per channel). Delay lines are allocated on
/// demand when a time parameter changes.
pub struct FxRuntime {
    sample_rate: f64,
    version: u64,
    cfg: TrackFx,
    buf: Vec<f32>,
    write: usize,
    lfo: f32,
    // de-hummer notches (rebuilt on config change)
    notches: Vec<Biquad>,
    debass: Biquad,
}

impl FxRuntime {
    pub fn new(sample_rate: f64) -> Self {
        FxRuntime {
            sample_rate,
            version: 0,
            cfg: TrackFx::default(),
            buf: Vec::new(),
            write: 0,
            lfo: 0.0,
            notches: Vec::new(),
            debass: Biquad::bypass(),
        }
    }

    pub fn reset(&mut self) {
        self.buf.fill(0.0);
        self.lfo = 0.0;
        for n in &mut self.notches {
            n.reset();
        }
        self.debass.reset();
    }

    fn ensure_buf(&mut self, frames: usize) {
        if self.buf.len() != frames {
            self.buf = vec![0.0; frames.max(1)];
            self.write = 0;
        }
    }

    pub fn process(&mut self, x: f32, state: &TrackFx, version: u64) -> f32 {
        if state.is_empty() {
            return x;
        }
        if self.version != version {
            self.cfg = state.clone();
            self.version = version;
            self.notches.clear();
            if let Some(h) = &self.cfg.de_hummer {
                if h.enabled {
                    let q = 8.0;
                    for n in 1..=4 {
                        let f = (h.base_hz * n as f64).min(self.sample_rate * 0.45);
                        self.notches.push(biquad_notch(f, q, self.sample_rate));
                    }
                }
            }
            self.debass = match &self.cfg.de_basser {
                Some(b) if b.enabled => {
                    biquad_low_shelf(b.freq_hz, b.gain_db, 0.8, self.sample_rate)
                }
                _ => Biquad::bypass(),
            };
        }
        let sr = self.sample_rate;
        let mut y = x;

        // De-Basser first (tonal balance in front of time FX).
        y = self.debass.process(y);

        // De-Hummer.
        for n in &mut self.notches {
            y = n.process(y);
        }

        // Distortion.
        if let Some(d) = &self.cfg.distortion {
            if d.enabled {
                let drive = d.drive.clamp(1.0, 20.0) as f32;
                let shaped = (y * drive).tanh() / drive.tanh();
                let mix = d.mix.clamp(0.0, 1.0) as f32;
                y = y * (1.0 - mix) + shaped * mix;
            }
        }

        // Chorus / flanger (modulated delay read).
        let mut modulated = None;
        if let Some(c) = &self.cfg.chorus {
            if c.enabled {
                modulated = Some((c.rate_hz, c.depth_ms, 0.0, c.mix));
            }
        }
        if let Some(f) = &self.cfg.flanger {
            if f.enabled {
                modulated = Some((f.rate_hz, f.depth_ms, f.feedback, f.mix));
            }
        }
        if let Some((rate, depth_ms, fb, mix)) = modulated {
            let base = if self.cfg.flanger.as_ref().map_or(false, |f| f.enabled) { 2.0 } else { 12.0 };
            let depth = (depth_ms.clamp(0.0, 15.0) / 1000.0 * sr as f64) as f32;
            let lfo_inc = 2.0 * std::f32::consts::PI * rate.clamp(0.05, 10.0) as f32 / sr as f32;
            self.lfo += lfo_inc;
            if self.lfo > 2.0 * std::f32::consts::PI {
                self.lfo -= 2.0 * std::f32::consts::PI;
            }
            let delay_frames = base + (self.lfo.sin() * 0.5 + 0.5) * depth;
            // 2-second shared ring covers the maximum modulated delay.
            self.ensure_buf(FX_MAX_DELAY);
            let read = (self.write as f32 - delay_frames + FX_MAX_DELAY as f32) as usize % FX_MAX_DELAY;
            let i0 = read;
            let i1 = (read + 1) % FX_MAX_DELAY;
            let frac = delay_frames.fract();
            let delayed = self.buf[i0] * (1.0 - frac) + self.buf[i1] * frac;
            let wet_in = y + delayed * fb.clamp(0.0, 0.95) as f32;
            self.buf[self.write] = wet_in;
            self.write = (self.write + 1) % FX_MAX_DELAY;
            let m = mix.clamp(0.0, 1.0) as f32;
            y = y * (1.0 - m * 0.5) + delayed * m;
        }

        // Delay / echo (feedback delay line). Resolve params as plain values
        // first (per-sample hot path — no clones, no held borrows).
        let (d_on, d_time, d_fb, d_mix, base_ms) = if let Some(d) = &self.cfg.echo {
            (d.enabled, d.time_ms, d.feedback, d.mix, 340.0)
        } else if let Some(d) = &self.cfg.delay {
            (d.enabled, d.time_ms, d.feedback, d.mix, 0.0)
        } else {
            (false, 0.0, 0.0, 0.0, 0.0)
        };
        if d_on {
            let time = (base_ms + d_time.clamp(1.0, 2000.0)).min(2000.0);
            let frames = (time / 1000.0 * sr).round() as usize;
            self.ensure_buf(FX_MAX_DELAY);
            let read = (self.write + FX_MAX_DELAY - frames.min(FX_MAX_DELAY - 1)) % FX_MAX_DELAY;
            let delayed = self.buf[read];
            let fb = d_fb.clamp(0.0, 0.95) as f32;
            self.buf[self.write] = y + delayed * fb;
            self.write = (self.write + 1) % FX_MAX_DELAY;
            let m = d_mix.clamp(0.0, 1.0) as f32;
            y = y * (1.0 - m) + delayed * m;
        }
        y
    }
}

// --- ITU-R BS.1770-4 loudness ----------------------------------------------

/// K-weighted loudness meter (BS.1770-4): stage-1 high shelf + stage-2 RLB
/// high-pass (exact 48 kHz coefficients), 400 ms blocks with 75 % overlap
/// (100 ms hop), absolute (−70 LUFS) + relative (−10 LU) gating for the
/// integrated value.
pub struct LoudnessMeter {
    shelf: Biquad,
    hp: Biquad,
    shelf2: Biquad,
    hp2: Biquad,
    /// Sum of squared K-weighted samples in the current 100 ms sub-block.
    acc: f64,
    acc_n: usize,
    /// Last four 100 ms sub-block mean squares (→ 400 ms momentary).
    recent: [f64; 4],
    /// 400 ms block mean squares at a 100 ms hop (gated integrated).
    blocks: Vec<f64>,
    sub_left: usize,
}

const SUB_LEN: usize = 4800; // 100 ms at 48 kHz

impl LoudnessMeter {
    pub fn new() -> Self {
        // ITU-R BS.1770-4 stage coefficients @ 48 kHz.
        let mut shelf = Biquad::bypass();
        let mut hp = Biquad::bypass();
        let mut shelf2 = Biquad::bypass();
        let mut hp2 = Biquad::bypass();
        shelf.load(1.53512485958697, -2.69169618940638, 1.19839281085285, -1.69065929318241, 0.73248077421585);
        hp.load(1.0, -2.0, 1.0, -1.99004745483398, 0.99007225036621);
        shelf2.load(1.53512485958697, -2.69169618940638, 1.19839281085285, -1.69065929318241, 0.73248077421585);
        hp2.load(1.0, -2.0, 1.0, -1.99004745483398, 0.99007225036621);
        LoudnessMeter {
            shelf,
            hp,
            shelf2,
            hp2,
            acc: 0.0,
            acc_n: 0,
            recent: [0.0; 4],
            blocks: Vec::new(),
            sub_left: SUB_LEN,
        }
    }

    pub fn reset(&mut self) {
        self.shelf.reset();
        self.hp.reset();
        self.shelf2.reset();
        self.hp2.reset();
        self.acc = 0.0;
        self.acc_n = 0;
        self.recent = [0.0; 4];
        self.blocks.clear();
        self.sub_left = SUB_LEN;
    }

    /// Process one stereo frame.
    pub fn process(&mut self, l: f32, r: f32) {
        let a = self.shelf.process(l);
        let b = self.hp.process(a);
        let c = self.shelf2.process(r);
        let d = self.hp2.process(c);
        self.acc += (b * b + d * d) as f64;
        self.acc_n += 2;
        self.sub_left -= 1;
        if self.sub_left == 0 {
            // One 100 ms sub-block finished.
            let sub_ms = if self.acc_n > 0 { self.acc / self.acc_n as f64 } else { 0.0 };
            self.recent.copy_within(0..3, 1);
            self.recent[0] = sub_ms;
            // 400 ms block at a 100 ms hop (75 % overlap).
            self.blocks.push(self.recent.iter().sum::<f64>() / 4.0);
            // Cap memory at ~8 hours of blocks.
            if self.blocks.len() > 288_000 {
                self.blocks.drain(..96_000);
            }
            self.acc = 0.0;
            self.acc_n = 0;
            self.sub_left = SUB_LEN;
        }
    }

    fn lufs(ms: f64) -> f64 {
        -0.691 + 10.0 * ms.max(1e-12).log10()
    }

    /// Momentary loudness (last 400 ms), LUFS.
    pub fn momentary(&self) -> f64 {
        Self::lufs(self.recent.iter().sum::<f64>() / 4.0)
    }

    /// Integrated (gated) loudness since the last reset, LUFS.
    pub fn integrated(&self) -> f64 {
        if self.blocks.is_empty() {
            return Self::lufs(0.0);
        }
        // Absolute gate.
        let above_abs: Vec<f64> = self.blocks.iter().copied().filter(|m| Self::lufs(*m) > -70.0).collect();
        if above_abs.is_empty() {
            return -70.0;
        }
        let mean_abs = above_abs.iter().sum::<f64>() / above_abs.len() as f64;
        let rel_thr = Self::lufs(mean_abs) - 10.0;
        let above_rel: Vec<f64> =
            above_abs.iter().copied().filter(|m| Self::lufs(*m) > rel_thr).collect();
        if above_rel.is_empty() {
            return rel_thr;
        }
        let mean = above_rel.iter().sum::<f64>() / above_rel.len() as f64;
        Self::lufs(mean)
    }
}

impl Default for LoudnessMeter {
    fn default() -> Self {
        Self::new()
    }
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

    // ---- v0.5.0 Fairlight --------------------------------------------------

    fn sine(freq: f64, i: usize, sr: f64, amp: f32) -> f32 {
        (2.0 * std::f64::consts::PI * freq * i as f64 / sr).sin() as f32 * amp
    }

    #[test]
    fn gate_closes_on_silence() {
        let mut st = DynamicsState::default();
        st.enabled = true;
        st.gate.enabled = true;
        st.gate.threshold_db = -30.0;
        st.gate.range_db = 40.0;
        st.gate.ratio = 3.0; // strong expansion for a clear test
        let mut rt = DynamicsRuntime::new(48_000.0);
        // 1 s of loud signal: gate must stay open.
        for i in 0..48_000 {
            rt.process(sine(1000.0, i, 48_000.0, 0.5), &st, 1);
        }
        assert!(rt.gr.gate_db > -3.0, "gate closed on loud signal: {}", rt.gr.gate_db);
        // 1 s of silence: gate must close (approach full range).
        let mut last = 0.0f32;
        for _ in 0..48_000 {
            rt.process(0.0, &st, 1);
            last = rt.gr.gate_db;
        }
        assert!(last < -20.0, "gate did not close on silence: {last}");
    }

    #[test]
    fn limiter_ceilings_peaks() {
        let mut st = DynamicsState::default();
        st.enabled = true;
        st.limiter.enabled = true;
        st.limiter.threshold_db = -6.0;
        let mut rt = DynamicsRuntime::new(48_000.0);
        let mut peak = 0.0f32;
        for i in 0..48_000 {
            let y = rt.process(sine(100.0, i, 48_000.0, 1.2), &st, 1);
            if i > 24_000 {
                peak = peak.max(y.abs());
            }
        }
        let peak_db = 20.0 * (peak as f64).log10();
        assert!(peak_db < -5.0, "limiter ceiling exceeded: {peak_db:.2} dB");
    }

    #[test]
    fn dynamics_transfer_matches_stages() {
        let mut st = DynamicsState::default();
        st.enabled = true;
        st.comp.enabled = true;
        st.comp.threshold_db = -20.0;
        st.comp.ratio = 2.0;
        st.comp.makeup_db = 0.0;
        // 0 dB in → −10 dB out (2:1 above −20).
        let out = dynamics_transfer_db(&st, 0.0);
        assert!((out - (-10.0)).abs() < 1e-6, "transfer {out}");
        // −40 dB in → unchanged (below threshold, no knee overlap at −20±3).
        let out2 = dynamics_transfer_db(&st, -40.0);
        assert!((out2 - (-40.0)).abs() < 1e-6, "transfer {out2}");
    }

    #[test]
    fn delay_lines_delay() {
        let mut fx = TrackFx::default();
        fx.delay = Some(DelayParams { enabled: true, time_ms: 100.0, feedback: 0.0, mix: 1.0 });
        let mut rt = FxRuntime::new(48_000.0);
        let mut got_at = None;
        for i in 0..8_000 {
            let x = if i < 100 { 1.0 } else { 0.0 };
            let y = rt.process(x, &fx, 1);
            if y.abs() > 0.4 && got_at.is_none() {
                got_at = Some(i);
            }
        }
        // 100 ms ≈ 4800 samples; allow tolerance for interpolation.
        let at = got_at.expect("delayed impulse never appeared");
        assert!((4800..=4900).contains(&at), "delayed impulse at {at}");
    }

    #[test]
    fn de_hummer_notches_50hz() {
        let mut fx = TrackFx::default();
        fx.de_hummer = Some(DeHummerParams { enabled: true, base_hz: 50.0, depth_db: 30.0 });
        let mut rt = FxRuntime::new(48_000.0);
        let mut last = 0.0f32;
        for i in 0..96_000 {
            last = rt.process(sine(50.0, i, 48_000.0, 0.5), &fx, 1);
        }
        assert!(last.abs() < 0.05, "50 Hz hum not attenuated: {last}");
    }

    #[test]
    fn fft_finds_the_tone() {
        let sr = 48_000.0;
        let n = 2048;
        let sig: Vec<f32> = (0..n).map(|i| sine(1000.0, i, sr, 0.8)).collect();
        let mut out = Vec::new();
        fft_spectrum_db(&sig, &mut out);
        assert_eq!(out.len(), n / 2);
        let bin = (1000.0 / sr * n as f64).round() as usize;
        let peak_bin = out
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        assert!((peak_bin as i64 - bin as i64).abs() <= 1, "peak bin {peak_bin} vs {bin}");
    }

    #[test]
    fn loudness_meter_reports_plausible_lufs() {
        // A 997 Hz sine at −20 dBFS per channel ≈ −23 LUFS (K-weighted ~0 dB
        // at 1 kHz). EBU tech 3341 test signal: −23 LUFS ± 0.1.
        let mut m = LoudnessMeter::new();
        let amp = 10f64.powf(-20.0 / 20.0) as f32;
        for i in 0..192_000 {
            let s = sine(997.0, i, 48_000.0, amp);
            m.process(s, s);
        }
        let lufs = m.integrated();
        assert!((-24.5..=-21.5).contains(&lufs), "integrated {lufs} LUFS");
        let mom = m.momentary();
        assert!((-24.5..=-21.5).contains(&mom), "momentary {mom} LUFS");
    }

    #[test]
    fn notch_and_shelf_helpers_work() {
        let n = biquad_notch(50.0, 8.0, 48_000.0);
        let low = n.magnitude_db(50.0, 48_000.0);
        assert!(low < -20.0, "notch at f0: {low}");
        let keep = n.magnitude_db(5000.0, 48_000.0);
        assert!(keep.abs() < 1.0, "notch away from f0: {keep}");
        let mut b = Biquad::bypass();
        b.load(1.0, 0.0, 0.0, 0.0, 0.0);
        assert_eq!(b.process(0.5), 0.5);
    }
}
