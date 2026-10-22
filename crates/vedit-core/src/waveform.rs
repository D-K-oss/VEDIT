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

/// Extract (or load from cache) audio peaks for an asset, 0..1 amplitude.
pub fn extract_peaks(ffmpeg: &Path, path: &Path, duration: f64, buckets: usize) -> Result<Vec<f32>> {
    let cache = peaks_cache_path(path, duration, buckets);
    if let Some(peaks) = load_cache(&cache, buckets) {
        return Ok(peaks);
    }
    let peaks = decode_peaks(ffmpeg, path, buckets)?;
    store_cache(&cache, &peaks);
    Ok(peaks)
}

fn decode_peaks(ffmpeg: &Path, path: &Path, buckets: usize) -> Result<Vec<f32>> {
    let mut cmd = base_command(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error"])
        .arg("-i")
        .arg(path)
        .args(["-map", "0:a:0?", "-ac", "1", "-ar", "8000", "-f", "f32le", "pipe:1"])
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
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    if samples.is_empty() {
        return Err(VeditError::Other("no audio decoded for waveform".into()));
    }

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
    // Light normalization so quiet-but-audible files still draw.
    let peak_max = peaks.iter().cloned().fold(0.0f32, f32::max);
    if peak_max > 1e-4 && peak_max < 0.9 {
        for p in &mut peaks {
            *p = (*p / peak_max).min(1.0);
        }
    }
    Ok(peaks)
}

fn load_cache(path: &Path, buckets: usize) -> Option<Vec<f32>> {
    let data = std::fs::read(path).ok()?;
    if data.len() != buckets * 4 {
        return None;
    }
    Some(
        data.chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
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
