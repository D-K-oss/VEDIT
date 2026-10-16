//! VEDIT core — media engine for the VEDIT video editor.
//!
//! Everything that talks to the outside world (ffmpeg/ffprobe, the project
//! file format, the timeline model and the export renderer) lives here so
//! that the GUI crate stays presentation-only.

pub mod error;
pub mod export;
pub mod ffmpeg;
pub mod media;
pub mod preview;
pub mod probe;
pub mod project;
pub mod thumbnail;
pub mod timeline;

pub use error::{Result, VeditError};
pub use media::{MediaAsset, MediaInfo, MediaKind};
pub use project::{Clip, Project, Track, TrackKind};
