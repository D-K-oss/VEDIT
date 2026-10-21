//! Real-time preview audio for VEDIT.
//!
//! Architecture (Windows / macOS):
//! * **Decode** — any media file's first audio stream is decoded ONCE to an
//!   in-memory stereo 48 kHz f32 buffer by ffmpeg (`audio_decode`). This is
//!   what finally fixed "no audio on Windows": rodio/symphonia only covered
//!   a subset of containers/codecs and panicked on some files. ffmpeg plays
//!   everything the user can import.
//! * **Mix** — a single rodio sink plays a custom `MixerSource` that runs a
//!   real software mixer on the audio thread: per-clip gains (fades +
//!   keyframes, pushed by the UI every frame), per-track buses with the
//!   6-band parametric EQ, the 3-band multiband compressor, pan, fader and
//!   solo/mute, plus the master fader. All of it is LIVE during editing.
//! * **Meters** — the mixer writes per-track / master peak levels (and
//!   compressor gain reduction) into atomics the UI polls for the meters.
//!
//! The Linux stub keeps the same API (CI builds have no audio device deps).

use vedit_core::dsp::{CompState, EqState};

/// One audible clip at the current playhead.
#[derive(Debug, Clone)]
#[allow(dead_code)] // fields are consumed by the native backend (Windows/macOS)
pub struct AudioClipSpec {
    pub clip_id: u64,
    pub asset_id: u64,
    /// Where to start inside the source file (seconds).
    pub source_pos: f64,
    /// Clip gain ONLY (base volume × fades × keyframes). Track fader, pan,
    /// EQ/comp and master are applied on the buses.
    pub volume: f64,
    pub speed: f64,
    /// Index into the track bus list (project.tracks order).
    pub track_index: usize,
}

/// UI-side per-track bus state, pushed every frame while playing.
#[derive(Debug, Clone)]
#[allow(dead_code)] // fields are consumed by the native backend (Windows/macOS)
pub struct TrackMix {
    pub fader: f64,
    pub pan: f64,
    pub audible: bool,
    pub eq: EqState,
    pub eq_version: u64,
    pub comp: CompState,
    pub comp_version: u64,
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod imp {
    use super::{AudioClipSpec, TrackMix};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use rodio::{OutputStream, Sink, Source};

    use vedit_core::audio_decode::{DecodedPcm, MIX_SAMPLE_RATE};
    use vedit_core::dsp::{CompRuntime, EqRuntime};

    /// One active clip as seen by the audio thread.
    #[derive(Debug, Clone)]
    struct ClipControl {
        clip_id: u64,
        asset_id: u64,
        track_index: usize,
        start_frame: f64,
        speed: f32,
        gain: f32,
    }

    struct EngineState {
        version: u64,
        playing: bool,
        master: f32,
        tracks: Vec<TrackMix>,
        clips: Vec<ClipControl>,
        pcm: HashMap<u64, Arc<DecodedPcm>>,
    }

    /// Shared between UI + audio threads.
    struct Shared {
        state: Mutex<EngineState>,
        /// Per-track peak level (f32 bits), swapped by the audio thread.
        track_levels: [AtomicU32; MAX_TRACKS],
        /// Per-track compressor gain reduction, mid band (f32 bits, dB ≤ 0).
        track_gr: [AtomicU32; MAX_TRACKS],
        master_level: AtomicU32,
        /// Monotonic config version (bumped by the UI on every change).
        cfg_version: AtomicU64,
    }

    impl Shared {
        fn new() -> Self {
            Shared {
                state: Mutex::new(EngineState {
                    version: 0,
                    playing: false,
                    master: 1.0,
                    tracks: Vec::new(),
                    clips: Vec::new(),
                    pcm: HashMap::new(),
                }),
                track_levels: std::array::from_fn(|_| AtomicU32::new(0)),
                track_gr: std::array::from_fn(|_| AtomicU32::new(0)),
                master_level: AtomicU32::new(0),
                cfg_version: AtomicU64::new(0),
            }
        }

        fn bump(&self) {
            self.cfg_version.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Max track buses (project track count beyond this is not metered).
    const MAX_TRACKS: usize = 32;

    /// A clip currently running on the audio thread (owns playback position).
    struct RunningClip {
        clip_id: u64,
        asset_id: u64,
        track_index: usize,
        pos: f64,
        speed: f32,
        gain: f32,
        gain_target: f32,
        pcm: Arc<DecodedPcm>,
    }

    /// Per-track audio-thread processing state.
    struct TrackRuntime {
        eq_l: EqRuntime,
        eq_r: EqRuntime,
        comp_l: CompRuntime,
        comp_r: CompRuntime,
        smooth_gain: f32,
    }

    impl TrackRuntime {
        fn new() -> Self {
            TrackRuntime {
                eq_l: EqRuntime::new(MIX_SAMPLE_RATE as f64),
                eq_r: EqRuntime::new(MIX_SAMPLE_RATE as f64),
                comp_l: CompRuntime::new(MIX_SAMPLE_RATE as f64),
                comp_r: CompRuntime::new(MIX_SAMPLE_RATE as f64),
                smooth_gain: 1.0,
            }
        }
    }

    /// The software mixer, driven as a rodio `Source` (interleaved stereo).
    struct MixerSource {
        shared: Arc<Shared>,
        /// Local copy of the control state (refreshed when version changes).
        local_version: u64,
        playing: bool,
        master: f32,
        clips: Vec<RunningClip>,
        track_cfg: Vec<TrackMix>,
        runtimes: Vec<TrackRuntime>,
        /// Stereo phase for interleaved output.
        phase: usize,
        /// Current frame (l, r) between the two `next()` calls.
        cur: (f32, f32),
    }

    impl MixerSource {
        fn refresh(&mut self) {
            let Ok(st) = self.shared.state.try_lock() else { return };
            if st.version == self.local_version {
                return;
            }
            self.local_version = st.version;
            let was_playing = self.playing;
            self.playing = st.playing;
            self.master = st.master;
            self.track_cfg = st.tracks.clone();
            if self.track_cfg.len() > MAX_TRACKS {
                self.track_cfg.truncate(MAX_TRACKS);
            }
            if !was_playing && self.playing {
                // Fresh start: drop everything, relaunch at the pushed
                // positions.
                self.clips.clear();
                for rt in &mut self.runtimes {
                    rt.eq_l.reset();
                    rt.eq_r.reset();
                    rt.comp_l.reset();
                    rt.comp_r.reset();
                }
            }
            if !self.playing {
                self.clips.clear();
            }
            // Sync the clip list: launch new, drop gone, update gains.
            let wanted = &st.clips;
            let mut next: Vec<RunningClip> = Vec::with_capacity(wanted.len());
            for c in wanted {
                if let Some(existing) = self.clips.iter_mut().find(|r| r.clip_id == c.clip_id) {
                    existing.gain_target = c.gain;
                    existing.speed = c.speed;
                    existing.track_index = c.track_index;
                    let moved = existing.clone_shallow(c);
                    next.push(moved);
                    continue;
                }
                let Some(pcm) = st.pcm.get(&c.asset_id).cloned() else {
                    continue; // PCM not decoded yet — will appear when ready
                };
                next.push(RunningClip {
                    clip_id: c.clip_id,
                    asset_id: c.asset_id,
                    track_index: c.track_index,
                    pos: c.start_frame,
                    speed: c.speed,
                    gain: c.gain,
                    gain_target: c.gain,
                    pcm,
                });
            }
            self.clips = next;
        }

        /// Process ONE stereo frame; returns (l, r).
        fn mix_frame(&mut self) -> (f32, f32) {
            let n_tracks = self.track_cfg.len();
            if self.runtimes.len() < n_tracks {
                self.runtimes.resize_with(n_tracks, TrackRuntime::new);
            }
            let mut buses = vec![(0.0f32, 0.0f32); n_tracks];

            // --- clip layer ---
            for clip in &mut self.clips {
                let frames = clip.pcm.frames() as f64;
                if clip.pos >= frames {
                    continue; // finished naturally
                }
                let i0 = clip.pos.floor().max(0.0) as usize;
                let frac = (clip.pos - i0 as f64) as f32;
                let ch = 2usize;
                let get = |idx: usize, ofs: usize| -> f32 {
                    let i = (idx * ch + ofs).min(clip.pcm.samples.len().saturating_sub(1));
                    clip.pcm.samples[i]
                };
                let i1 = (i0 + 1).min(clip.pcm.frames().saturating_sub(1));
                let l = get(i0, 0) * (1.0 - frac) + get(i1, 0) * frac;
                let r = get(i0, 1) * (1.0 - frac) + get(i1, 1) * frac;
                // Per-sample gain smoothing (anti-zipper).
                clip.gain += (clip.gain_target - clip.gain) * 0.004;
                let ti = clip.track_index.min(MAX_TRACKS - 1);
                if ti < buses.len() {
                    let b = &mut buses[ti];
                    b.0 += l * clip.gain;
                    b.1 += r * clip.gain;
                }
                clip.pos += clip.speed as f64;
            }

            // --- track buses: EQ → comp → fader/pan ---
            let mut mix_l = 0.0f32;
            let mut mix_r = 0.0f32;
            for (ti, bus) in buses.iter_mut().enumerate() {
                let (Some(cfg), Some(rt)) = (self.track_cfg.get(ti), self.runtimes.get_mut(ti)) else {
                    continue;
                };
                let mut l = bus.0;
                let mut r = bus.1;
                if cfg.audible {
                    l = rt.eq_l.process(l, &cfg.eq, cfg.eq_version);
                    r = rt.eq_r.process(r, &cfg.eq, cfg.eq_version);
                    l = rt.comp_l.process(l, &cfg.comp, cfg.comp_version);
                    r = rt.comp_r.process(r, &cfg.comp, cfg.comp_version);
                    // Fader with smooth ramp + equal-power pan.
                    let target = cfg.fader as f32;
                    rt.smooth_gain += (target - rt.smooth_gain) * 0.002;
                    let p = cfg.pan.clamp(-1.0, 1.0) as f32;
                    let angle = (p + 1.0) * 0.25 * std::f32::consts::PI; // -1..1 → 0..π/2
                    let (gl, gr) = (angle.cos(), angle.sin());
                    l *= rt.smooth_gain * gl;
                    r *= rt.smooth_gain * gr;
                } else {
                    rt.smooth_gain += (0.0 - rt.smooth_gain) * 0.002;
                    l *= rt.smooth_gain;
                    r *= rt.smooth_gain;
                }
                // Meter: peak of this frame.
                let peak = l.abs().max(r.abs()).min(1.5);
                self.shared.track_levels[ti].store(peak.to_bits(), Ordering::Relaxed);
                // GR meter (mid band) every ~21 ms.
                self.shared.track_gr[ti]
                    .store(rt.comp_l.gr_mid_db.to_bits(), Ordering::Relaxed);
                mix_l += l;
                mix_r += r;
            }

            // --- master ---
            let m = self.master;
            mix_l *= m;
            mix_r *= m;
            let peak = mix_l.abs().max(mix_r.abs()).min(1.5);
            self.shared.master_level.store(peak.to_bits(), Ordering::Relaxed);
            (mix_l.clamp(-1.6, 1.6), mix_r.clamp(-1.6, 1.6))
        }
    }

    impl RunningClip {
        fn clone_shallow(&self, c: &ClipControl) -> RunningClip {
            RunningClip {
                clip_id: self.clip_id,
                asset_id: self.asset_id,
                track_index: c.track_index,
                pos: self.pos,
                speed: c.speed,
                gain: self.gain,
                gain_target: c.gain,
                pcm: self.pcm.clone(),
            }
        }
    }

    impl Iterator for MixerSource {
        type Item = f32;
        fn next(&mut self) -> Option<f32> {
            if self.phase == 0 {
                self.refresh();
                let (l, r) = if self.playing { self.mix_frame() } else { (0.0, 0.0) };
                self.cur = (l, r);
                self.phase = 1;
                Some(l)
            } else {
                self.phase = 0;
                Some(self.cur.1)
            }
        }
    }

    impl Source for MixerSource {
        fn current_frame_len(&self) -> Option<usize> {
            Some(2)
        }
        fn channels(&self) -> u16 {
            2
        }
        fn sample_rate(&self) -> u32 {
            MIX_SAMPLE_RATE
        }
        fn total_duration(&self) -> Option<std::time::Duration> {
            None
        }
    }

    pub struct AudioPreviewer {
        shared: Arc<Shared>,
        _stream: Option<OutputStream>,
        _sink: Option<Sink>,
        /// Version of the last pushed track config (to avoid needless bumps).
        last_push_version: u64,
    }

    impl AudioPreviewer {
        pub fn new() -> Self {
            AudioPreviewer {
                shared: Arc::new(Shared::new()),
                _stream: None,
                _sink: None,
                last_push_version: 0,
            }
        }

        /// Open the output stream lazily (first play). Never panics on
        /// failure — preview audio just stays silent.
        fn ensure_running(&mut self) -> bool {
            if self._sink.is_some() {
                return true;
            }
            let Ok((stream, handle)) = OutputStream::try_default() else {
                log::warn!("no audio output device — preview will be silent");
                return false;
            };
            let src = MixerSource {
                shared: self.shared.clone(),
                local_version: u64::MAX, // force initial refresh
                playing: false,
                master: 1.0,
                clips: Vec::new(),
                track_cfg: Vec::new(),
                runtimes: Vec::new(),
                phase: 0,
                cur: (0.0, 0.0),
            };
            let Ok(sink) = Sink::try_new(&handle) else {
                log::warn!("cannot open audio sink — preview will be silent");
                return false;
            };
            sink.append(src);
            // Keep the sink alive for the app's lifetime (dropping it stops
            // the source and closes the stream).
            self._stream = Some(stream);
            self._sink = Some(sink);
            true
        }

        /// Insert (or replace) decoded PCM for an asset.
        pub fn insert_pcm(&self, asset_id: u64, pcm: Arc<DecodedPcm>) {
            if let Ok(mut st) = self.shared.state.lock() {
                st.pcm.insert(asset_id, pcm);
                st.version += 1;
            }
            self.shared.bump();
        }

        /// Start playback with the given clip list.
        pub fn start(&mut self, specs: Vec<AudioClipSpec>, tracks: Vec<TrackMix>, master: f64) {
            if !self.ensure_running() {
                return;
            }
            self.push_state(specs, tracks, master, true);
        }

        /// Refresh gains/positions while playing (called every UI frame).
        pub fn update(&mut self, specs: &[AudioClipSpec], tracks: Vec<TrackMix>, master: f64) {
            if self._sink.is_none() {
                return;
            }
            self.push_state(specs.to_vec(), tracks, master, true);
        }

        /// Stop playback (output goes silent; clips cleared).
        pub fn stop(&mut self) {
            if let Ok(mut st) = self.shared.state.lock() {
                st.playing = false;
                st.clips.clear();
                st.version += 1;
            }
            self.shared.bump();
        }

        fn push_state(&mut self, specs: Vec<AudioClipSpec>, tracks: Vec<TrackMix>, master: f64, playing: bool) {
            let clips = specs
                .into_iter()
                .map(|s| ClipControl {
                    clip_id: s.clip_id,
                    asset_id: s.asset_id,
                    track_index: s.track_index,
                    start_frame: s.source_pos.max(0.0) * MIX_SAMPLE_RATE as f64,
                    speed: s.speed.max(0.01) as f32,
                    gain: s.volume.clamp(0.0, 4.0) as f32,
                })
                .collect();
            if let Ok(mut st) = self.shared.state.lock() {
                st.playing = playing;
                st.master = master.clamp(0.0, 2.0) as f32;
                st.tracks = tracks;
                st.clips = clips;
                st.version += 1;
            }
            self.shared.bump();
            let _ = self.last_push_version;
        }

        /// Decayed peak level for a track bus (0..1+), for the UI meters.
        pub fn track_level(&self, index: usize) -> f32 {
            if index >= MAX_TRACKS {
                return 0.0;
            }
            f32::from_bits(self.shared.track_levels[index].load(Ordering::Relaxed))
        }

        pub fn master_level(&self) -> f32 {
            f32::from_bits(self.shared.master_level.load(Ordering::Relaxed))
        }

        /// Mid-band compressor gain reduction for a track (dB, ≤ 0).
        pub fn track_gr(&self, index: usize) -> f32 {
            if index >= MAX_TRACKS {
                return 0.0;
            }
            f32::from_bits(self.shared.track_gr[index].load(Ordering::Relaxed))
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod imp {
    use super::{AudioClipSpec, TrackMix};

    /// Stub used on Linux (no live audio preview; exports are unaffected).
    pub struct AudioPreviewer;

    impl AudioPreviewer {
        pub fn new() -> Self {
            Self
        }
        pub fn insert_pcm(&self, _asset_id: u64, _pcm: std::sync::Arc<vedit_core::audio_decode::DecodedPcm>) {}
        pub fn start(&mut self, _specs: Vec<AudioClipSpec>, _tracks: Vec<TrackMix>, _master: f64) {}
        pub fn update(&mut self, _specs: &[AudioClipSpec], _tracks: Vec<TrackMix>, _master: f64) {}
        pub fn stop(&mut self) {}
        pub fn track_level(&self, _i: usize) -> f32 {
            0.0
        }
        pub fn master_level(&self) -> f32 {
            0.0
        }
        pub fn track_gr(&self, _i: usize) -> f32 {
            0.0
        }
    }
}

pub use imp::AudioPreviewer;
