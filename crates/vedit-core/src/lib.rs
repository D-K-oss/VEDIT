//! VEDIT core — media engine for the VEDIT video editor.
//!
//! Everything that talks to the outside world (ffmpeg/ffprobe, the project
//! file format, the timeline model and the export renderer) lives here so
//! that the GUI crate stays presentation-only.

pub mod audio_decode;
pub mod colorgrade;
pub mod dsp;
pub mod error;
pub mod export;
pub mod ffmpeg;
pub mod lut;
pub mod media;
pub mod preview;
pub mod probe;
pub mod project;
pub mod proxy;
pub mod textrender;
pub mod thumbnail;
pub mod timeline;
pub mod warp;
pub mod waveform;

pub use colorgrade::GradeTransform;
pub use error::{Result, VeditError};
pub use lut::Lut3d;
pub use media::{MediaAsset, MediaInfo, MediaKind};
pub use project::{
    BlendMode, Clip, ClipTransform, ColorGrade, Crop, Curve, Keyframe, Project, TextClipData,
    Track, TrackKind, Transition, TransitionKind, Wheel,
};
