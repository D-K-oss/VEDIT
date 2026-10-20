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

fn default_one() -> f64 {
    1.0
}

// ---------------------------------------------------------------------------
// Effects model (v0.2.0)
// ---------------------------------------------------------------------------

/// Kind of transition rendered between a clip and the NEXT clip on the same
/// track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionKind {
    CrossDissolve,
}

impl TransitionKind {
    pub fn label(&self) -> &'static str {
        match self {
            TransitionKind::CrossDissolve => "Cross-dissolve",
        }
    }
}

/// A transition attached to the END of a clip (its relationship with the
/// clip that follows on the same track).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub kind: TransitionKind,
    /// Nominal overlap length in timeline seconds. The effective overlap is
    /// clamped against both clips (see `timeline::effective_overlap`).
    pub duration: f64,
}

impl Default for Transition {
    fn default() -> Self {
        Transition {
            kind: TransitionKind::CrossDissolve,
            duration: 0.5,
        }
    }
}

/// One keyframe on a clip. `t` is clip-local time (seconds from the clip
/// start). Both properties live on the same keyframe so one list drives
/// opacity and volume together (like Premiere's combined clip keyframes).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub t: f64,
    pub opacity: f64,
    pub volume: f64,
}

/// Payload of a text/title clip (these clips have no media asset).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextClipData {
    pub content: String,
    /// Font size in pixels at project resolution.
    pub size: f64,
    /// RGBA color.
    pub color: [u8; 4],
    pub bold: bool,
    /// 0.0 = left, 0.5 = centered, 1.0 = right (relative to the anchor).
    pub align: f64,
    /// Normalized anchor position inside the frame (0..1).
    pub x: f64,
    pub y: f64,
    /// Line-height multiplier.
    pub line_spacing: f64,
}

impl Default for TextClipData {
    fn default() -> Self {
        TextClipData {
            content: "Title".to_string(),
            size: 96.0,
            color: [255, 255, 255, 255],
            bold: true,
            align: 0.5,
            x: 0.5,
            y: 0.5,
            line_spacing: 1.2,
        }
    }
}

/// Parametric color grade + optional .cube 3D LUT. Every parameter is
/// linear/affine, so preview and export can share one transform model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorGrade {
    /// Exposure adjustment in stops (-2..2).
    pub exposure: f64,
    /// Contrast (-1..1, 0 = neutral).
    pub contrast: f64,
    /// Saturation (0..2, 1 = neutral).
    pub saturation: f64,
    /// Temperature (-1..1, cool ↔ warm).
    pub temperature: f64,
    /// Tint (-1..1, magenta ↔ green).
    pub tint: f64,
    /// Optional .cube LUT applied after the parametric grade.
    #[serde(default)]
    pub lut_path: Option<PathBuf>,
}

impl Default for ColorGrade {
    fn default() -> Self {
        ColorGrade {
            exposure: 0.0,
            contrast: 0.0,
            saturation: 1.0,
            temperature: 0.0,
            tint: 0.0,
            lut_path: None,
        }
    }
}

impl ColorGrade {
    pub fn is_neutral(&self) -> bool {
        self.exposure.abs() < 1e-4
            && self.contrast.abs() < 1e-4
            && (self.saturation - 1.0).abs() < 1e-4
            && self.temperature.abs() < 1e-4
            && self.tint.abs() < 1e-4
            && self.lut_path.is_none()
    }

    pub fn lut_name(&self) -> Option<String> {
        self.lut_path
            .as_ref()
            .map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.display().to_string())
            })
    }
}

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
    /// Id of the linked A/V partner clip (video ↔ audio twin sharing one
    /// source file). `None` for unlinked clips and older project files.
    #[serde(default)]
    pub link: Option<u64>,
    /// Fade-in length (timeline seconds): video opacity + audio volume ramp.
    #[serde(default)]
    pub fade_in: f64,
    /// Fade-out length (timeline seconds).
    #[serde(default)]
    pub fade_out: f64,
    /// Transition rendered into the clip that FOLLOWS on the same track.
    #[serde(default)]
    pub transition_out: Option<Transition>,
    /// Opacity/volume keyframes, sorted by `t` (clip-local seconds).
    #[serde(default)]
    pub keyframes: Vec<Keyframe>,
    /// Text/title payload. `Some(..)` means this is a text clip (no asset).
    #[serde(default)]
    pub text: Option<TextClipData>,
    /// Per-clip color grade (None = neutral).
    #[serde(default)]
    pub grade: Option<ColorGrade>,
}

/// Piecewise-linear curve of (time, value) points, time ascending.
pub type Curve = Vec<(f64, f64)>;

fn interp_keyframes(kf: &[Keyframe], t: f64, pick: impl Fn(&Keyframe) -> f64, base: f64) -> f64 {
    if kf.is_empty() {
        return base;
    }
    if t <= kf[0].t {
        return pick(&kf[0]);
    }
    let last = &kf[kf.len() - 1];
    if t >= last.t {
        return pick(last);
    }
    for w in kf.windows(2) {
        if t >= w[0].t && t <= w[1].t {
            let span = (w[1].t - w[0].t).max(1e-9);
            let f = (t - w[0].t) / span;
            return pick(&w[0]) * (1.0 - f) + pick(&w[1]) * f;
        }
    }
    base
}

/// Reduce a set of curve sample times to a sorted, unique, in-range list.
fn curve_times(cands: &mut Vec<f64>, dur: f64) {
    cands.retain(|t| *t >= -1e-9 && *t <= dur + 1e-9);
    cands.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    cands.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    if cands.len() < 2 {
        *cands = vec![0.0, dur.max(0.0)];
        if dur <= 1e-9 {
            cands.pop();
        }
    }
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

    /// True when this clip renders text instead of media.
    pub fn is_text(&self) -> bool {
        self.text.is_some()
    }

    /// Linear fade ramp 0..1 at clip-local time `local_t`.
    /// `extra_fade_out` extends/overrides the fade-out window (used by
    /// cross-dissolves on the outgoing clip).
    pub fn fade_ramp_at(&self, local_t: f64, extra_fade_out: f64) -> f64 {
        let mut r = 1.0f64;
        if self.fade_in > 1e-6 && local_t < self.fade_in {
            r *= (local_t / self.fade_in).clamp(0.0, 1.0);
        }
        let fo = if extra_fade_out > 1e-6 {
            extra_fade_out
        } else {
            self.fade_out
        };
        if fo > 1e-6 {
            let t0 = self.duration - fo;
            if local_t > t0 {
                r *= ((self.duration - local_t) / fo).clamp(0.0, 1.0);
            }
        }
        r.clamp(0.0, 1.0)
    }

    /// Final opacity at clip-local time: keyframed opacity × fade ramp.
    pub fn opacity_at(&self, local_t: f64, extra_fade_out: f64) -> f64 {
        let base = interp_keyframes(&self.keyframes, local_t, |k| k.opacity, self.opacity);
        (base * self.fade_ramp_at(local_t, extra_fade_out)).clamp(0.0, 1.0)
    }

    /// Final clip volume at clip-local time: keyframed volume × fade ramp.
    pub fn volume_at(&self, local_t: f64) -> f64 {
        let base = interp_keyframes(&self.keyframes, local_t, |k| k.volume, self.volume);
        (base * self.fade_ramp_at(local_t, 0.0)).clamp(0.0, 4.0)
    }

    /// Piecewise-linear opacity curve over [0, duration], folding base
    /// opacity, keyframes, fades and an optional extra fade-out window
    /// (cross-dissolve). Shared by preview and export so both agree.
    pub fn opacity_curve(&self, extra_fade_out: f64) -> Curve {
        let dur = self.duration.max(0.0);
        let mut ts = vec![0.0, dur];
        if self.fade_in > 1e-6 {
            ts.push(self.fade_in);
        }
        let fo = if extra_fade_out > 1e-6 { extra_fade_out } else { self.fade_out };
        if fo > 1e-6 {
            ts.push((dur - fo).max(0.0));
        }
        for k in &self.keyframes {
            ts.push(k.t);
        }
        curve_times(&mut ts, dur);
        ts.iter()
            .map(|t| (*t, self.opacity_at(*t, extra_fade_out)))
            .collect()
    }

    /// Piecewise-linear volume curve over [0, duration] (clip-local).
    pub fn volume_curve(&self) -> Curve {
        let dur = self.duration.max(0.0);
        let mut ts = vec![0.0, dur];
        if self.fade_in > 1e-6 {
            ts.push(self.fade_in);
        }
        if self.fade_out > 1e-6 {
            ts.push((dur - self.fade_out).max(0.0));
        }
        for k in &self.keyframes {
            ts.push(k.t);
        }
        curve_times(&mut ts, dur);
        ts.iter().map(|t| (*t, self.volume_at(*t))).collect()
    }

    /// Evaluate a piecewise-linear curve.
    pub fn eval_curve(curve: &Curve, t: f64) -> f64 {
        if curve.is_empty() {
            return 1.0;
        }
        if t <= curve[0].0 {
            return curve[0].1;
        }
        let last = curve[curve.len() - 1];
        if t >= last.0 {
            return last.1;
        }
        for w in curve.windows(2) {
            if t >= w[0].0 && t <= w[1].0 {
                let span = (w[1].0 - w[0].0).max(1e-9);
                let f = (t - w[0].0) / span;
                return w[0].1 * (1.0 - f) + w[1].1 * f;
            }
        }
        last.1
    }

    /// Classify an opacity curve for the export filtergraph:
    /// constant → `aa=`, simple fade(s) → `fade` filters, anything else
    /// (keyframes) → per-frame alpha expression.
    pub fn curve_shape(curve: &Curve, dur: f64) -> CurveShape {
        if curve.len() <= 1 {
            return CurveShape::Flat(curve.first().map(|p| p.1).unwrap_or(1.0));
        }
        // Linear ramp from (0,v0) to (dur,v1)? → fade filter territory.
        if curve.len() == 2
            && curve[0].0.abs() < 1e-6
            && (curve[1].0 - dur).abs() < 1e-3
            && curve[0].1.abs() < 1e-3
            && (curve[1].1 - 1.0).abs() < 1e-3
        {
            return CurveShape::FadeIn(dur);
        }
        if curve.len() == 2
            && curve[0].0.abs() < 1e-6
            && (curve[1].0 - dur).abs() < 1e-3
            && (curve[0].1 - 1.0).abs() < 1e-3
            && curve[1].1.abs() < 1e-3
        {
            return CurveShape::FadeOut(dur);
        }
        if curve.len() == 3
            && curve[0].0.abs() < 1e-6
            && curve[0].1.abs() < 1e-3
            && (curve[1].1 - 1.0).abs() < 1e-3
            && curve[2].0 > curve[1].0 + 1e-3
            && (curve[2].0 - dur).abs() < 1e-3
            && curve[2].1.abs() < 1e-3
        {
            return CurveShape::FadeInOut(curve[1].0, dur - curve[2].0);
        }
        // Constant value with plateaus?
        let v = curve[0].1;
        if curve.iter().all(|p| (p.1 - v).abs() < 1e-3) {
            return CurveShape::Flat(v);
        }
        CurveShape::Keyframed
    }
}

/// Shape of an opacity/volume curve, used to pick the cheapest ffmpeg filter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CurveShape {
    /// Single constant value.
    Flat(f64),
    /// Linear 0→1 ramp over the full duration.
    FadeIn(f64),
    /// Linear 1→0 ramp over the full duration.
    FadeOut(f64),
    /// (peak_time, fade_out_len) — 0→1→0 triangle.
    FadeInOut(f64, f64),
    /// Anything else: needs per-frame expressions.
    Keyframed,
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
    // ---- mixer (v0.2.0, all back-compatible defaults) ----
    /// Track fader (0..2, 1.0 = unity).
    #[serde(default = "default_one")]
    pub volume: f64,
    /// Stereo pan (-1 = left, 0 = center, 1 = right). Export-side.
    #[serde(default)]
    pub pan: f64,
    /// Solo: when any track is soloed, only solo tracks are audible.
    #[serde(default)]
    pub solo: bool,
    /// 3-band EQ gains in dB (-12..12). Export-side.
    #[serde(default)]
    pub eq_bass: f64,
    #[serde(default)]
    pub eq_mid: f64,
    #[serde(default)]
    pub eq_treble: f64,
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
    /// Master output fader (0..2, 1.0 = unity).
    #[serde(default = "default_one")]
    pub master_volume: f64,
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
            master_volume: 1.0,
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
            volume: 1.0,
            pan: 0.0,
            solo: false,
            eq_bass: 0.0,
            eq_mid: 0.0,
            eq_treble: 0.0,
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

    /// Locate the linked partner of the clip `(track_id, clip_id)`.
    /// Returns the partner's `(track_id, clip_id)`.
    pub fn find_linked(&self, track_id: u64, clip_id: u64) -> Option<(u64, u64)> {
        let link = self.find_clip(track_id, clip_id)?.link?;
        for t in &self.tracks {
            if t.clips.iter().any(|c| c.id == link) {
                return Some((t.id, link));
            }
        }
        None
    }

    /// Clear `link` fields that point to removed clip ids (dangling links).
    pub fn clear_links_to(&mut self, removed: &[u64]) {
        if removed.is_empty() {
            return;
        }
        for t in &mut self.tracks {
            for c in &mut t.clips {
                if c.link.map(|l| removed.contains(&l)).unwrap_or(false) {
                    c.link = None;
                }
            }
        }
    }

    /// Remove an asset and every clip that references it.
    /// Returns how many clips were removed.
    pub fn remove_asset(&mut self, asset_id: u64) -> usize {
        let mut removed_ids = Vec::new();
        for t in &mut self.tracks {
            t.clips.retain(|c| {
                if c.asset_id == asset_id {
                    removed_ids.push(c.id);
                    false
                } else {
                    true
                }
            });
        }
        self.clear_links_to(&removed_ids);
        self.assets.retain(|a| a.id != asset_id);
        removed_ids.len()
    }

    pub fn remove_track(&mut self, track_id: u64) -> bool {
        let before = self.tracks.len();
        let removed_ids: Vec<u64> = self
            .tracks
            .iter()
            .filter(|t| t.id == track_id)
            .flat_map(|t| t.clips.iter().map(|c| c.id))
            .collect();
        self.tracks.retain(|t| t.id != track_id);
        let removed = self.tracks.len() != before;
        if removed {
            self.clear_links_to(&removed_ids);
        }
        removed
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
