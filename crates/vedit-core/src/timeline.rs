//! Pure timeline operations shared by GUI interactions (drag, trim, split…).
//!
//! All functions clamp against neighbouring clips so that clips never
//! overlap on a track.

use crate::project::{Clip, Project, TrackKind};

/// Largest timeline length a single image clip may take (sanity cap).
pub const MAX_IMAGE_CLIP_DUR: f64 = 60.0 * 15.0;
pub const MIN_CLIP_DUR: f64 = 0.05;

/// Clamp a proposed `start` for a clip of length `dur` on `track` so it does
/// not overlap neighbours. `ignore` is the clip being moved (it may span its
/// own current slot).
pub fn clamp_placement(track: &crate::project::Track, ignore: Option<u64>, start: f64, dur: f64) -> f64 {
    let mut lo = 0.0f64;
    let mut hi = f64::INFINITY;
    for c in &track.clips {
        if Some(c.id) == ignore {
            continue;
        }
        if c.end() <= start {
            lo = lo.max(c.end());
        } else if c.start >= start + dur {
            hi = hi.min(c.start);
        } else {
            // Overlapping neighbour: choose the closer side to push out of.
            let dist_left = (start - c.end()).abs();
            let dist_right = (c.start - start).abs();
            if dist_left < dist_right {
                lo = lo.max(c.end());
            } else {
                hi = hi.min(c.start);
            }
        }
    }
    start.clamp(lo, (hi - dur).max(lo))
}

/// Snap `t` to the closest candidate within `tol`; returns Some(snapped).
pub fn snap_time(candidates: &[f64], t: f64, tol: f64) -> Option<f64> {
    if tol <= 0.0 {
        return None;
    }
    let mut best: Option<(f64, f64)> = None; // (delta, value)
    for &c in candidates {
        let d = (c - t).abs();
        if d <= tol && best.map(|(bd, _)| d < bd).unwrap_or(true) {
            best = Some((d, c));
        }
    }
    best.map(|(_, v)| v)
}

/// Default clip duration for an asset (images get 5 s).
pub fn default_clip_duration(asset: &crate::media::MediaAsset) -> f64 {
    match asset.kind {
        crate::media::MediaKind::Image => 5.0,
        _ => asset.duration().unwrap_or(5.0).max(MIN_CLIP_DUR),
    }
}

/// Create a clip from an asset and drop it on `track_id` at `start`
/// (clamped). Returns the new clip id.
pub fn add_asset_clip(project: &mut Project, asset_id: u64, track_id: u64, start: f64) -> crate::error::Result<u64> {
    let (clip_dur, in_point, speed) = {
        let asset = project
            .asset(asset_id)
            .ok_or_else(|| crate::error::VeditError::other("asset not found"))?;
        let dur = default_clip_duration(asset);
        (dur, 0.0, 1.0)
    };
    let track = project
        .track(track_id)
        .ok_or_else(|| crate::error::VeditError::other("track not found"))?;
    let start = clamp_placement(track, None, start, clip_dur);
    let clip = Clip {
        id: project.alloc_id(),
        asset_id,
        start,
        in_point,
        duration: clip_dur,
        speed,
        volume: 1.0,
        opacity: 1.0,
        muted: false,
    };
    let id = clip.id;
    project.track_mut(track_id).unwrap().insert_clip(clip);
    Ok(id)
}

/// Move a clip (possibly to another track of the same kind). Returns true if
/// the track changed.
pub fn move_clip(project: &mut Project, from_track: u64, clip_id: u64, to_track: u64, new_start: f64) -> bool {
    let Some(mut clip) = project.track(from_track).and_then(|t| t.find_clip(clip_id)).cloned() else {
        return false;
    };
    let dur = clip.duration;
    let Some(track) = project.track(to_track) else {
        return false;
    };
    if track.locked {
        return false;
    }
    clip.start = clamp_placement(track, Some(clip_id), new_start.max(0.0), dur);
    if to_track == from_track {
        if let Some(t) = project.track_mut(from_track) {
            if let Some(c) = t.find_clip_mut(clip_id) {
                c.start = clip.start;
            }
            sort_track(t);
        }
        false
    } else {
        // Same kind only.
        let from_kind = project.track(from_track).map(|t| t.kind);
        let to_kind = project.track(to_track).map(|t| t.kind);
        if from_kind != to_kind {
            return false;
        }
        project.track_mut(from_track).unwrap().clips.retain(|c| c.id != clip_id);
        project.track_mut(to_track).unwrap().insert_clip(clip);
        true
    }
}

fn sort_track(track: &mut crate::project::Track) {
    track.clips.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
}

/// Trim the left edge to `new_start` (timeline seconds). Shifts `in_point`.
pub fn trim_start(project: &mut Project, track_id: u64, clip_id: u64, new_start: f64) {
    let Some(clip) = project.find_clip(track_id, clip_id).cloned() else {
        return;
    };
    let asset_len = project.asset(clip.asset_id).and_then(|a| a.duration());
    // How early can we start? in_point >= 0 → cannot move left past
    // start - in_point/speed. For images there is no source limit.
    let min_start = if asset_len.is_some() {
        clip.start - clip.in_point / clip.speed
    } else {
        0.0
    };
    let max_start = clip.end() - MIN_CLIP_DUR;
    let new_start = new_start.clamp(min_start.max(0.0), max_start);
    let delta = new_start - clip.start; // negative when trimming left
    let Some(track) = project.track_mut(track_id) else {
        return;
    };
    if let Some(c) = track.find_clip_mut(clip_id) {
        c.in_point = (c.in_point + delta * c.speed).max(0.0);
        c.start = new_start;
        c.duration -= delta;
    }
    sort_track(track);
}

/// Trim the right edge to `new_end` (timeline seconds).
pub fn trim_end(project: &mut Project, track_id: u64, clip_id: u64, new_end: f64) {
    let Some(clip) = project.find_clip(track_id, clip_id).cloned() else {
        return;
    };
    let asset_len = project.asset(clip.asset_id).and_then(|a| a.duration());
    let max_end = match asset_len {
        Some(len) => clip.start + (len - clip.in_point) / clip.speed,
        None => clip.start + MAX_IMAGE_CLIP_DUR,
    };
    let min_end = clip.start + MIN_CLIP_DUR;
    let new_end = new_end.clamp(min_end, max_end);
    if let Some(track) = project.track_mut(track_id) {
        if let Some(c) = track.find_clip_mut(clip_id) {
            c.duration = new_end - c.start;
        }
        sort_track(track);
    }
}

/// Split the clip under `time` on `track_id` at exactly `time`.
/// Returns the id of the new (right-hand) clip.
pub fn split_at(project: &mut Project, track_id: u64, time: f64) -> Option<u64> {
    let track = project.track(track_id)?;
    let clip = track.clip_at(time)?.clone();
    if time - clip.start < MIN_CLIP_DUR || clip.end() - time < MIN_CLIP_DUR {
        return None;
    }
    let left_dur = time - clip.start;
    let mut right = clip.clone();
    right.id = project.alloc_id();
    right.start = time;
    right.duration = clip.duration - left_dur;
    right.in_point = clip.in_point + left_dur * clip.speed;
    let right_id = right.id;
    let track = project.track_mut(track_id)?;
    if let Some(c) = track.find_clip_mut(clip.id) {
        c.duration = left_dur;
    }
    track.insert_clip(right);
    Some(right_id)
}

/// Split whatever clips intersect `time` on every unlocked track.
/// Returns ids of created clips.
pub fn split_all_at(project: &mut Project, time: f64) -> Vec<u64> {
    let track_ids: Vec<u64> = project
        .tracks
        .iter()
        .filter(|t| !t.locked && t.clip_at(time).is_some())
        .map(|t| t.id)
        .collect();
    let mut created = Vec::new();
    for tid in track_ids {
        if let Some(id) = split_at(project, tid, time) {
            created.push(id);
        }
    }
    created
}

pub fn remove_clip(project: &mut Project, track_id: u64, clip_id: u64) -> bool {
    let Some(track) = project.track_mut(track_id) else {
        return false;
    };
    let before = track.clips.len();
    track.clips.retain(|c| c.id != clip_id);
    track.clips.len() != before
}

/// First unlocked track of `kind`, preferring `preferred_id`.
pub fn find_target_track(project: &Project, kind: TrackKind, preferred_id: Option<u64>) -> Option<u64> {
    if let Some(id) = preferred_id {
        if let Some(t) = project.track(id) {
            if t.kind == kind && !t.locked {
                return Some(id);
            }
        }
    }
    project
        .tracks
        .iter()
        .find(|t| t.kind == kind && !t.locked)
        .map(|t| t.id)
}
