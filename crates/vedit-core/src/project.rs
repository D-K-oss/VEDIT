//! The VEDIT project model: assets, tracks, clips + JSON persistence.
//!
//! Coordinates are always in **seconds** (f64) on the *timeline*, while clips
//! also remember their `in_point` in *source* seconds. `clip.speed` relates
//! the two: `source_duration = timeline_duration * speed`.

use std::path::Path;
use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::media::MediaAsset;

pub const PROJECT_FILE_EXT: &str = "vedit.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackKind {
    Video,
    Audio,
}

impl TrackKind {
    pub fn label(&self) -> &'static str {
        match self {
            TrackKind::Video => "VIDEO",
            TrackKind::Audio => "AUDIO",
        }
    }
}

/// One clip instance on a timeline track.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    pub id: u64,
    pub asset_id: u64,
    /// Timeline position (seconds).
    pub start: f64,
    /// Offset inside the source file (seconds).
    pub in_point: f64,
    /// Length on the timeline (seconds).
    pub duration: f64,
    /// Playback rate (1.0 = normal). source_duration = duration * speed.
    pub speed: f64,
    pub volume: f64,
    pub opacity: f64,
    pub muted: bool,
}

impl Clip {
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }

    /// How much of the source file this clip consumes.
    pub fn source_duration(&self) -> f64 {
        self.duration * self.speed
    }

    /// Last source second used.
    pub fn source_end(&self) -> f64 {
        self.in_point + self.source_duration()
    }

    /// Source time that corresponds to a timeline position within the clip.
    pub fn source_time_at(&self, timeline_t: f64) -> f64 {
        self.in_point + (timeline_t - self.start).max(0.0) * self.speed
    }
}

/// A timeline track. `clips` is always kept sorted by `start`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: u64,
    pub kind: TrackKind,
    pub name: String,
    pub muted: bool,
    pub locked: bool,
    pub clips: Vec<Clip>,
}

impl Track {
    pub fn clip_at(&self, t: f64) -> Option<&Clip> {
        self.clips.iter().find(|c| t >= c.start && t < c.end())
    }

    pub fn find_clip(&self, id: u64) -> Option<&Clip> {
        self.clips.iter().find(|c| c.id == id)
    }

    pub fn find_clip_mut(&mut self, id: u64) -> Option<&mut Clip> {
        self.clips.iter_mut().find(|c| c.id == id)
    }

    /// Insert keeping sort order (stable for identical starts).
    pub fn insert_clip(&mut self, clip: Clip) {
        let pos = self
            .clips
            .iter()
            .position(|c| c.start > clip.start)
            .unwrap_or(self.clips.len());
        self.clips.insert(pos, clip);
    }

    /// End of the last clip on this track.
    pub fn end(&self) -> f64 {
        self.clips.last().map(|c| c.end()).unwrap_or(0.0)
    }
}

/// Root of a `.vedit.json` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub next_id: u64,
    pub assets: Vec<MediaAsset>,
    /// Display order top → bottom. Video tracks composite bottom → top.
    pub tracks: Vec<Track>,
}

impl Default for Project {
    fn default() -> Self {
        Project::new("Untitled Project", 1920, 1080, 30.0)
    }
}

impl Project {
    /// Fresh project with a Premiere-like default track layout.
    /// Display order (top→bottom): V2, V1, A1, A2.
    pub fn new(name: &str, width: u32, height: u32, fps: f64) -> Self {
        let mut p = Project {
            name: name.to_string(),
            width,
            height,
            fps,
            next_id: 1,
            assets: Vec::new(),
            tracks: Vec::new(),
        };
        let v2 = p.add_track_named(TrackKind::Video, "V2");
        let v1 = p.add_track_named(TrackKind::Video, "V1");
        let a1 = p.add_track_named(TrackKind::Audio, "A1");
        let a2 = p.add_track_named(TrackKind::Audio, "A2");
        p.tracks = vec![v2, v1, a1, a2];
        p
    }

    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn add_track_named(&mut self, kind: TrackKind, name: &str) -> Track {
        let id = self.alloc_id();
        Track {
            id,
            kind,
            name: name.to_string(),
            muted: false,
            locked: false,
            clips: Vec::new(),
        }
    }

    /// Add a new video track (displayed on top) or audio track (bottom).
    pub fn add_track(&mut self, kind: TrackKind) -> u64 {
        let n_video = self.tracks.iter().filter(|t| t.kind == TrackKind::Video).count();
        let n_audio = self.tracks.iter().filter(|t| t.kind == TrackKind::Audio).count();
        let track = match kind {
            TrackKind::Video => self.add_track_named(kind, &format!("V{}", n_video + 1)),
            TrackKind::Audio => self.add_track_named(kind, &format!("A{}", n_audio + 1)),
        };
        let id = track.id;
        match kind {
            TrackKind::Video => self.tracks.insert(0, track),
            TrackKind::Audio => self.tracks.push(track),
        }
        id
    }

    pub fn track(&self, id: u64) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: u64) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    pub fn asset(&self, id: u64) -> Option<&MediaAsset> {
        self.assets.iter().find(|a| a.id == id)
    }

    /// End of the whole timeline (last clip end across tracks).
    pub fn duration(&self) -> f64 {
        self.tracks.iter().map(|t| t.end()).fold(0.0, f64::max)
    }

    pub fn clip_count(&self) -> usize {
        self.tracks.iter().map(|t| t.clips.len()).sum()
    }

    pub fn find_clip(&self, track_id: u64, clip_id: u64) -> Option<&Clip> {
        self.track(track_id).and_then(|t| t.find_clip(clip_id))
    }

    /// Remove an asset and every clip that references it.
    /// Returns how many clips were removed.
    pub fn remove_asset(&mut self, asset_id: u64) -> usize {
        let mut removed = 0;
        for t in &mut self.tracks {
            let before = t.clips.len();
            t.clips.retain(|c| c.asset_id != asset_id);
            removed += before - t.clips.len();
        }
        self.assets.retain(|a| a.id != asset_id);
        removed
    }

    pub fn remove_track(&mut self, track_id: u64) -> bool {
        let before = self.tracks.len();
        self.tracks.retain(|t| t.id != track_id);
        self.tracks.len() != before
    }

    // ---- persistence -------------------------------------------------

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let file = fs::File::create(path)?;
        serde_json::to_writer_pretty(file, self)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<(Self, PathBuf)> {
        let data = fs::read(path)?;
        let project: Project = serde_json::from_slice(&data)?;
        Ok((project, path.to_path_buf()))
    }
}
