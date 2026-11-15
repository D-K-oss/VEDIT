//! Decode any media file's first audio stream to in-memory f32 PCM.
//!
//! The preview mixer needs seekable, sample-accurate audio for *any* format
//! the user imports (MOV/MP4/AAC, MKV/DTS, whatever their ffmpeg supports).
//! symphonia (via rodio) covered only a subset and panicked on some files —
//! so instead we decode once with ffmpeg into a stereo 48 kHz f32 buffer and
//! mix from memory. 48 kHz stereo f32 is ~380 KB per second; buffers are
//! capped (`MAX_DECODE_SECONDS`) so an hour-long music file can't eat all RAM.

use std::io::{BufReader, Read};
use std::path::Path;
use std::process::{ChildStdout, Stdio};

use crate::error::{Result, VeditError};
use crate::ffmpeg::{base_command, kill_child};

/// Output sample rate of every decoded buffer (all mixing happens at 48 kHz).
pub const MIX_SAMPLE_RATE: u32 = 48_000;
/// Channels: preview mixes stereo.
pub const MIX_CHANNELS: usize = 2;
/// Hard cap for a single decoded asset (seconds).
pub const MAX_DECODE_SECONDS: f64 = 900.0;

/// Decoded, interleaved stereo f32 samples at `MIX_SAMPLE_RATE`.
#[derive(Debug, Clone)]
pub struct DecodedPcm {
    /// Interleaved L,R,L,R…
    pub samples: std::sync::Arc<Vec<f32>>,
    pub sample_rate: u32,
    pub channels: u32,
    /// True when the source was longer than the cap and got truncated.
    pub truncated: bool,
}

impl DecodedPcm {
    /// Frame count (per-channel samples).
    pub fn frames(&self) -> usize {
        self.samples.len() / MIX_CHANNELS.max(1)
    }

    /// Duration in seconds.
    pub fn duration(&self) -> f64 {
        self.frames() as f64 / self.sample_rate.max(1) as f64
    }

    /// Peak absolute amplitude (for Normalize).
    pub fn peak(&self) -> f32 {
        self.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    /// RMS amplitude.
    pub fn rms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = self.samples.iter().map(|s| (*s as f64) * (*s as f64)).sum();
        (sum / self.samples.len() as f64).sqrt() as f32
    }
}

/// Spawn ffmpeg and decode the first audio stream to f32le stereo 48 kHz.
pub fn decode_pcm(ffmpeg: &Path, path: &Path) -> Result<DecodedPcm> {
    let mut cmd = base_command(ffmpeg);
    cmd.arg("-v").arg("error");
    cmd.arg("-i").arg(path);
    cmd.arg("-vn");
    cmd.arg("-map").arg("0:a:0?");
    cmd.arg("-ac").arg(MIX_CHANNELS.to_string());
    cmd.arg("-ar").arg(MIX_SAMPLE_RATE.to_string());
    cmd.arg("-f").arg("f32le");
    cmd.arg("-acodec").arg("pcm_f32le");
    cmd.arg("pipe:1");
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| VeditError::Other(format!("cannot start ffmpeg for audio decode: {e}")))?;
    let stdout: ChildStdout = child.stdout.take().expect("stdout piped");
    let cap_bytes =
        (MAX_DECODE_SECONDS * MIX_SAMPLE_RATE as f64 * MIX_CHANNELS as f64 * 4.0) as usize;
    let mut reader = BufReader::with_capacity(1 << 20, stdout);
    let mut samples: Vec<f32> = Vec::with_capacity(cap_bytes / 4 / 4);
    let mut buf = [0u8; 64 * 1024];
    let mut truncated = false;
    loop {
        // Read in chunks; stop when the cap is hit.
        let want = cap_bytes.min(buf.len());
        match reader.read(&mut buf[..want]) {
            Ok(0) => break,
            Ok(n) => {
                let floats = n / 4;
                let chunk: Vec<f32> = buf[..floats * 4]
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                samples.extend_from_slice(&chunk);
                if samples.len() * 4 >= cap_bytes {
                    truncated = true;
                    break;
                }
            }
            Err(e) => {
                kill_child(&mut child);
                return Err(VeditError::Other(format!("audio decode read failed: {e}")));
            }
        }
    }
    kill_child(&mut child);

    if samples.is_empty() {
        return Err(VeditError::Other("no audio stream (or silent decode)".into()));
    }
    Ok(DecodedPcm {
        samples: std::sync::Arc::new(samples),
        sample_rate: MIX_SAMPLE_RATE,
        channels: MIX_CHANNELS as u32,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::FfmpegLocator;

    /// Generate a 1 s 440 Hz sine wav with ffmpeg and decode it.
    #[test]
    fn decodes_sine() {
        let Ok(locator) = FfmpegLocator::discover() else {
            eprintln!("ffmpeg not available; skipping");
            return;
        };
        let dir = std::env::temp_dir().join(format!("vedit-pcm-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("sine.wav");
        let st = std::process::Command::new(locator.ffmpeg())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
            .arg(&wav)
            .status()
            .unwrap();
        assert!(st.success());
        let pcm = decode_pcm(locator.ffmpeg(), &wav).expect("decode");
        assert!((pcm.duration() - 1.0).abs() < 0.1, "dur {}", pcm.duration());
        // ffmpeg's `sine` source runs at ~-21 dBFS (amplitude ≈ 0.088).
        assert!(pcm.peak() > 0.05 && pcm.peak() <= 1.0, "peak {}", pcm.peak());
        assert!(pcm.rms() > 0.03, "rms {}", pcm.rms());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
