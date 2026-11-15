//! Audio waveform peak extraction for timeline clip rendering.
//!
//! Decodes the first audio stream to mono f32 via a piped ffmpeg process and
//! reduces it to `buckets` peak amplitudes (0..1). Results are cached on
//! disk, keyed by a hash of path + duration + bucket count.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::error::{Result, VeditError};
use crate::ffmpeg::base_command;

/// FNV-1a 64-bit — stable cache keys across runs.
pub fn fnv64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn peaks_cache_dir() -> PathBuf {
    crate::thumbnail::cache_dir().join("peaks")
}

fn peaks_cache_path(path: &Path, duration: f64, buckets: usize) -> PathBuf {
    let key = fnv64(
        format!(
            "{}|{:.3}|{}",
            path.display(),
            duration,
            buckets
        )
        .as_bytes(),
    );
    peaks_cache_dir().join(format!("peaks-{key:x}.bin"))
}

/// Number of peaks stored per asset (covers ~40 min at 0.6 s resolution…
/// plenty for any zoom level; drawing interpolates).
pub const DEFAULT_BUCKETS: usize = 4000;

/// Per-channel audio peaks for timeline rendering (Resolve-style stereo
/// waveform: two mirrored envelopes). Values are 0..1 amplitudes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StereoPeaks {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl StereoPeaks {
    /// Mono fold-down (average of the channels) for callers that only need
    /// a single envelope.
    pub fn mono(&self) -> Vec<f32> {
        if self.left.is_empty() && self.right.is_empty() {
            return Vec::new();
        }
        if self.right.is_empty() {
            return self.left.clone();
        }
        if self.left.is_empty() {
            return self.right.clone();
        }
        self.left
            .iter()
            .zip(self.right.iter())
            .map(|(l, r)| (l + r) * 0.5)
            .collect()
    }
}

/// Extract (or load from cache) STEREO audio peaks for an asset.
/// Falls back to a mono-derived result when the file has one channel.
pub fn extract_peaks_stereo(
    ffmpeg: &Path,
    path: &Path,
    duration: f64,
    buckets: usize,
) -> Result<StereoPeaks> {
    let cache = stereo_cache_path(path, duration, buckets);
    if let Some(peaks) = load_cache_stereo(&cache, buckets) {
        return Ok(peaks);
    }
    let peaks = decode_peaks_stereo(ffmpeg, path, buckets)?;
    store_cache_stereo(&cache, &peaks);
    Ok(peaks)
}

fn stereo_cache_path(path: &Path, duration: f64, buckets: usize) -> PathBuf {
    let key = fnv64(
        format!("stereo2|{}|{:.3}|{}", path.display(), duration, buckets).as_bytes(),
    );
    peaks_cache_dir().join(format!("peaks-{key:x}.bin"))
}

fn decode_peaks_stereo(ffmpeg: &Path, path: &Path, buckets: usize) -> Result<StereoPeaks> {
    let samples = decode_f32le(ffmpeg, path, 2, buckets)?;
    let (left, right) = split_interleaved(&samples, 2);
    let mut peaks = StereoPeaks {
        left: reduce_peaks(&left, buckets),
        right: reduce_peaks(&right, buckets),
    };
    normalize_peaks(&mut peaks.left);
    normalize_peaks(&mut peaks.right);
    Ok(peaks)
}

/// Decode the first audio stream as interleaved f32 LE samples.
/// `channels` is passed to ffmpeg's `-ac`; returns an empty Vec on failure.
fn decode_f32le(ffmpeg: &Path, path: &Path, channels: u8, _buckets: usize) -> Result<Vec<f32>> {
    let mut cmd = base_command(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error"])
        .arg("-i")
        .arg(path)
        .args([
            "-map",
            "0:a:0?",
            "-ac",
            &channels.to_string(),
            "-ar",
            "8000",
            "-f",
            "f32le",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|e| VeditError::Other(format!("failed to launch ffmpeg for waveform: {e}")))?;
    let mut out = child.stdout.take().expect("stdout piped");
    let mut bytes = Vec::with_capacity(64 * 1024);
    let _ = out.read_to_end(&mut bytes);
    let _ = child.wait();

    let samples: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect();
    if samples.is_empty() {
        return Err(VeditError::Other("no audio decoded for waveform".into()));
    }
    Ok(samples)
}

/// Split interleaved samples into per-channel vectors.
/// A single-channel input maps entirely onto the left channel.
pub(crate) fn split_interleaved(samples: &[f32], channels: u8) -> (Vec<f32>, Vec<f32>) {
    match channels {
        2 => {
            let mut left = Vec::with_capacity(samples.len() / 2);
            let mut right = Vec::with_capacity(samples.len() / 2);
            for (i, s) in samples.iter().enumerate() {
                if i % 2 == 0 {
                    left.push(*s);
                } else {
                    right.push(*s);
                }
            }
            (left, right)
        }
        _ => (samples.to_vec(), Vec::new()),
    }
}

/// Reduce raw samples to `buckets` peak amplitudes (0..1).
pub(crate) fn reduce_peaks(samples: &[f32], buckets: usize) -> Vec<f32> {
    let mut peaks = vec![0.0f32; buckets];
    let per = (samples.len() / buckets).max(1);
    for (i, peak) in peaks.iter_mut().enumerate() {
        let start = i * per;
        if start >= samples.len() {
            break;
        }
        let end = (start + per).min(samples.len());
        let mut max = 0.0f32;
        for s in &samples[start..end] {
            let a = s.abs();
            if a > max {
                max = a;
            }
        }
        *peak = max.min(1.0);
    }
    peaks
}

/// Light normalization so quiet-but-audible files still draw.
pub(crate) fn normalize_peaks(peaks: &mut [f32]) {
    let peak_max = peaks.iter().cloned().fold(0.0f32, f32::max);
    if peak_max > 1e-4 && peak_max < 0.9 {
        for p in peaks.iter_mut() {
            *p = (*p / peak_max).min(1.0);
        }
    }
}

/// Extract (or load from cache) audio peaks for an asset, 0..1 amplitude.
pub fn extract_peaks(ffmpeg: &Path, path: &Path, duration: f64, buckets: usize) -> Result<Vec<f32>> {
    let cache = peaks_cache_path(path, duration, buckets);
    if let Some(peaks) = load_cache(&cache, buckets) {
        return Ok(peaks);
    }
    let samples = decode_f32le(ffmpeg, path, 1, buckets)?;
    let mut peaks = reduce_peaks(&samples, buckets);
    normalize_peaks(&mut peaks);
    store_cache(&cache, &peaks);
    Ok(peaks)
}

fn load_cache(path: &Path, buckets: usize) -> Option<Vec<f32>> {
    let data = std::fs::read(path).ok()?;
    if data.len() != buckets * 4 {
        return None;
    }
    Some(
        data.as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect(),
    )
}

fn store_cache(path: &Path, peaks: &[f32]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut bytes = Vec::with_capacity(peaks.len() * 4);
    for p in peaks {
        bytes.extend_from_slice(&p.to_le_bytes());
    }
    let _ = std::fs::write(path, bytes);
}

/// On-disk stereo layout: interleaved f32 LE pairs (l0, r0, l1, r1, …).
fn load_cache_stereo(path: &Path, buckets: usize) -> Option<StereoPeaks> {
    let data = std::fs::read(path).ok()?;
    if data.len() != buckets * 8 {
        return None;
    }
    let vals: Vec<f32> = data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect();
    let (left, right) = split_interleaved(&vals, 2);
    Some(StereoPeaks { left, right })
}

fn store_cache_stereo(path: &Path, peaks: &StereoPeaks) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if peaks.left.len() != peaks.right.len() {
        return; // refuse to persist a malformed entry
    }
    let mut bytes = Vec::with_capacity(peaks.left.len() * 8);
    for (l, r) in peaks.left.iter().zip(peaks.right.iter()) {
        bytes.extend_from_slice(&l.to_le_bytes());
        bytes.extend_from_slice(&r.to_le_bytes());
    }
    let _ = std::fs::write(path, bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_interleaved_stereo_and_mono() {
        let samples = [0.1, 0.9, 0.2, 0.8, 0.3, 0.7];
        let (l, r) = split_interleaved(&samples, 2);
        assert_eq!(l, vec![0.1, 0.2, 0.3]);
        assert_eq!(r, vec![0.9, 0.8, 0.7]);
        let (m, empty) = split_interleaved(&samples, 1);
        assert_eq!(m, samples.to_vec());
        assert!(empty.is_empty());
    }

    #[test]
    fn reduce_peaks_finds_maxima_and_handles_silence() {
        let loud = vec![0.0, 0.5, -1.0, 0.25];
        let peaks = reduce_peaks(&loud, 2);
        assert_eq!(peaks.len(), 2);
        assert!((peaks[0] - 0.5).abs() < 1e-6);
        assert!((peaks[1] - 1.0).abs() < 1e-6);
        // Silence stays silent.
        let silent = reduce_peaks(&vec![0.0; 128], 8);
        assert!(silent.iter().all(|p| *p == 0.0));
    }

    #[test]
    fn normalize_peaks_only_lifts_quiet_material() {
        let mut quiet = vec![0.05, 0.10, 0.02];
        normalize_peaks(&mut quiet);
        assert!((quiet[1] - 1.0).abs() < 1e-6);
        // Already-loud material is untouched.
        let mut loud = vec![0.95, 0.4];
        let before = loud.clone();
        normalize_peaks(&mut loud);
        assert_eq!(loud, before);
    }

    #[test]
    fn stereo_mono_folddown() {
        let st = StereoPeaks { left: vec![0.2, 0.4], right: vec![0.6, 0.8] };
        assert_eq!(st.mono(), vec![0.4, 0.6]);
        let l_only = StereoPeaks { left: vec![0.3], right: vec![] };
        assert_eq!(l_only.mono(), vec![0.3]);
        let empty = StereoPeaks::default();
        assert!(empty.mono().is_empty());
    }

    #[test]
    fn stereo_cache_roundtrip() {
        let dir = std::env::temp_dir().join(format!("vedit-peaks-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("cache.bin");
        let peaks = StereoPeaks {
            left: vec![0.1; 16],
            right: vec![0.9; 16],
        };
        store_cache_stereo(&path, &peaks);
        let loaded = load_cache_stereo(&path, 16).expect("cache should load");
        assert_eq!(loaded, peaks);
        // Wrong bucket count must be rejected, not misread.
        assert!(load_cache_stereo(&path, 8).is_none());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
}
