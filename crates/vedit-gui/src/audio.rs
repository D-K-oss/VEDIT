//! Audio preview playback (Windows / macOS). Linux stub returns no-ops —
//! exports always carry audio, only live preview playback is platform-gated.

use std::path::PathBuf;

/// One audible clip at the current playhead.
/// (Fields are consumed by the native backend; the Linux stub ignores them.)
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AudioClipSpec {
    pub clip_id: u64,
    pub path: PathBuf,
    /// Where to start inside the source file (seconds).
    pub source_pos: f64,
    pub volume: f64,
    pub speed: f64,
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod imp {
    use super::AudioClipSpec;
    use std::collections::HashMap;
    use std::io::BufReader;

    use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};

    pub struct AudioPreviewer {
        _stream: Option<OutputStream>,
        handle: Option<OutputStreamHandle>,
        sinks: HashMap<u64, Sink>,
        playing: bool,
    }

    impl AudioPreviewer {
        pub fn new() -> Self {
            match OutputStream::try_default() {
                Ok((stream, handle)) => Self {
                    _stream: Some(stream),
                    handle: Some(handle),
                    sinks: HashMap::new(),
                    playing: false,
                },
                Err(_) => Self {
                    _stream: None,
                    handle: None,
                    sinks: HashMap::new(),
                    playing: false,
                },
            }
        }

        pub fn start(&mut self, specs: Vec<AudioClipSpec>) {
            self.stop();
            self.playing = true;
            for spec in specs {
                self.launch(spec);
            }
        }

        fn launch(&mut self, spec: AudioClipSpec) {
            let Some(handle) = &self.handle else { return };
            let Ok(file) = std::fs::File::open(&spec.path) else { return };
            let Ok(source) = Decoder::new(BufReader::new(file)) else {
                log::debug!("audio preview: cannot decode {}", spec.path.display());
                return;
            };
            let Ok(sink) = Sink::try_new(handle) else { return };
            sink.append(source);
            let _ = sink.try_seek(std::time::Duration::from_secs_f64(spec.source_pos.max(0.0)));
            sink.set_volume(spec.volume.clamp(0.0, 2.0) as f32);
            self.sinks.insert(spec.clip_id, sink);
        }

        /// Called every UI frame while playing: launch newly active clips,
        /// stop clips that are no longer active.
        pub fn update(&mut self, specs: &[AudioClipSpec]) {
            if !self.playing {
                return;
            }
            // Drop finished sinks.
            self.sinks.retain(|_, sink| !sink.empty());
            // Stop sinks whose clip is no longer under the playhead.
            let stale: Vec<u64> = self
                .sinks
                .keys()
                .filter(|id| !specs.iter().any(|s| s.clip_id == *id))
                .cloned()
                .collect();
            for id in stale {
                if let Some(mut sink) = self.sinks.remove(&id) {
                    sink.stop();
                }
            }
            // Launch clips that just became active.
            for spec in specs {
                if !self.sinks.contains_key(&spec.clip_id) {
                    self.launch(spec.clone());
                }
            }
        }

        pub fn stop(&mut self) {
            self.playing = false;
            for (_, mut sink) in self.sinks.drain() {
                sink.stop();
            }
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod imp {
    use super::AudioClipSpec;

    /// Stub used on Linux (no live audio preview; exports are unaffected).
    pub struct AudioPreviewer;

    impl AudioPreviewer {
        pub fn new() -> Self {
            Self
        }
        pub fn start(&mut self, _specs: Vec<AudioClipSpec>) {}
        pub fn update(&mut self, _specs: &[AudioClipSpec]) {}
        pub fn stop(&mut self) {}
    }
}

pub use imp::AudioPreviewer;
