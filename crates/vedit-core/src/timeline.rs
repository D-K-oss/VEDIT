//! Pure timeline operations shared by GUI interactions (drag, trim, split…).
//!
//! All functions clamp against neighbouring clips so that clips never
//! overlap on a track.

use crate::project::{Clip, Keyframe, Project, TextClipData, TrackKind};

/// Largest timeline length a single image clip may take (sanity cap).
pub const MAX_IMAGE_CLIP_DUR: f64 = 60.0 * 15.0;
pub const MIN_CLIP_DUR: f64 = 0.05;
pub const MAX_FADE: f64 = 10.0;
pub const MAX_TRANSITION: f64 = 5.0;

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
        link: None,
        fade_in: 0.0,
        fade_out: 0.0,
        transition_out: None,
        keyframes: Vec::new(),
        text: None,
        grade: None,
    };
    let id = clip.id;
    project.track_mut(track_id).unwrap().insert_clip(clip);
    Ok(id)
}

/// Create a clip from an asset and drop it on `track_id` at `start`.
///
/// Premiere-style linked A/V: a **video** asset that carries audio is added
/// as a pair — the video half lands on `track_id`, an audio twin lands on
/// the first free audio track, and both clips reference each other via
/// `Clip::link` so every later edit keeps them in sync.
///
/// Returns the created `(track_id, clip_id)` pairs (video first).
pub fn add_asset_clip_auto(
    project: &mut Project,
    asset_id: u64,
    track_id: u64,
    start: f64,
) -> crate::error::Result<Vec<(u64, u64)>> {
    let (clip_dur, is_video, has_audio) = {
        let asset = project
            .asset(asset_id)
            .ok_or_else(|| crate::error::VeditError::other("asset not found"))?;
        (
            default_clip_duration(asset),
            asset.kind == crate::media::MediaKind::Video,
            asset.has_audio(),
        )
    };
    {
        let track = project
            .track(track_id)
            .ok_or_else(|| crate::error::VeditError::other("track not found"))?;
        if track.locked {
            return Err(crate::error::VeditError::other("track is locked"));
        }
    }
    let start = {
        let track = project.track(track_id).unwrap();
        clamp_placement(track, None, start, clip_dur)
    };
    let vid = project.alloc_id();
    project.track_mut(track_id).unwrap().insert_clip(Clip {
        id: vid,
        asset_id,
        start,
        in_point: 0.0,
        duration: clip_dur,
        speed: 1.0,
        volume: 1.0,
        opacity: 1.0,
        muted: false,
        link: None,
        fade_in: 0.0,
        fade_out: 0.0,
        transition_out: None,
        keyframes: Vec::new(),
        text: None,
        grade: None,
    });
    let mut made = vec![(track_id, vid)];

    // Linked audio twin for video sources that carry sound.
    if is_video && has_audio {
        // Prefer an audio track where the twin fits without colliding.
        let audio_tracks: Vec<u64> = project
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio && !t.locked)
            .map(|t| t.id)
            .collect();
        let chosen = audio_tracks
            .iter()
            .copied()
            .find(|&tid| {
                let t = project.track(tid).unwrap();
                clamp_placement(t, None, start, clip_dur) == start
            })
            .or_else(|| audio_tracks.first().copied());
        if let Some(atid) = chosen {
            let twin_start = {
                let t = project.track(atid).unwrap();
                clamp_placement(t, None, start, clip_dur)
            };
            let aid = project.alloc_id();
            project.track_mut(atid).unwrap().insert_clip(Clip {
                id: aid,
                asset_id,
                start: twin_start,
                in_point: 0.0,
                duration: clip_dur,
                speed: 1.0,
                volume: 1.0,
                opacity: 1.0,
                muted: false,
                link: Some(vid),
                fade_in: 0.0,
                fade_out: 0.0,
                transition_out: None,
                keyframes: Vec::new(),
                text: None,
                grade: None,
            });
            if let Some(t) = project.track_mut(track_id) {
                if let Some(c) = t.find_clip_mut(vid) {
                    c.link = Some(aid);
                }
            }
            made.push((atid, aid));
        }
    }
    Ok(made)
}

/// Move a clip (possibly to another track of the same kind). Linked A/V
/// partners follow by the same time delta. Returns true if the track changed.
pub fn move_clip(project: &mut Project, from_track: u64, clip_id: u64, to_track: u64, new_start: f64) -> bool {
    let Some(mut clip) = project.track(from_track).and_then(|t| t.find_clip(clip_id)).cloned() else {
        return false;
    };
    let old_start = clip.start;
    let dur = clip.duration;
    let Some(track) = project.track(to_track) else {
        return false;
    };
    if track.locked {
        return false;
    }
    clip.start = clamp_placement(track, Some(clip_id), new_start.max(0.0), dur);
    let delta = clip.start - old_start;
    if to_track == from_track {
        if let Some(t) = project.track_mut(from_track) {
            if let Some(c) = t.find_clip_mut(clip_id) {
                c.start = clip.start;
            }
            sort_track(t);
        }
        sync_link_move(project, from_track, clip_id, delta);
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
        sync_link_move(project, to_track, clip_id, delta);
        true
    }
}

/// Shift the linked partner of a moved clip by the same time delta.
fn sync_link_move(project: &mut Project, track_id: u64, clip_id: u64, delta: f64) {
    if delta.abs() < 1e-9 {
        return;
    }
    let Some((pt, pc)) = project.find_linked(track_id, clip_id) else {
        return;
    };
    if project.track(pt).map(|t| t.locked).unwrap_or(true) {
        return;
    }
    let Some(p) = project.track(pt).and_then(|t| t.find_clip(pc)).cloned() else {
        return;
    };
    let new_start = {
        let t = project.track(pt).unwrap();
        clamp_placement(t, Some(pc), (p.start + delta).max(0.0), p.duration)
    };
    if let Some(t) = project.track_mut(pt) {
        if let Some(c) = t.find_clip_mut(pc) {
            c.start = new_start;
        }
        sort_track(t);
    }
}

fn sort_track(track: &mut crate::project::Track) {
    track.clips.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
}

/// Trim the left edge to `new_start` (timeline seconds). Shifts `in_point`.
/// The linked A/V partner (if any) is trimmed by the same delta.
pub fn trim_start(project: &mut Project, track_id: u64, clip_id: u64, new_start: f64) {
    if project.track(track_id).map(|t| t.locked).unwrap_or(true) {
        return;
    }
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
    sync_link_trim_start(project, track_id, clip_id, delta);
}

/// Mirror a left-trim onto the linked partner.
fn sync_link_trim_start(project: &mut Project, track_id: u64, clip_id: u64, delta: f64) {
    if delta.abs() < 1e-9 {
        return;
    }
    let Some((pt, pc)) = project.find_linked(track_id, clip_id) else {
        return;
    };
    if project.track(pt).map(|t| t.locked).unwrap_or(true) {
        return;
    }
    let Some(p) = project.track(pt).and_then(|t| t.find_clip(pc)).cloned() else {
        return;
    };
    let asset_len = project.asset(p.asset_id).and_then(|a| a.duration());
    let min_start = if asset_len.is_some() {
        p.start - p.in_point / p.speed
    } else {
        0.0
    };
    let max_start = p.end() - MIN_CLIP_DUR;
    let ns = (p.start + delta).clamp(min_start.max(0.0), max_start);
    let d = ns - p.start;
    if let Some(t) = project.track_mut(pt) {
        if let Some(c) = t.find_clip_mut(pc) {
            c.in_point = (c.in_point + d * c.speed).max(0.0);
            c.start = ns;
            c.duration -= d;
        }
        sort_track(t);
    }
}

/// Trim the right edge to `new_end` (timeline seconds).
/// The linked A/V partner (if any) is trimmed by the same delta.
pub fn trim_end(project: &mut Project, track_id: u64, clip_id: u64, new_end: f64) {
    if project.track(track_id).map(|t| t.locked).unwrap_or(true) {
        return;
    }
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
    let old_end = clip.end();
    let delta = new_end - old_end;
    if let Some(track) = project.track_mut(track_id) {
        if let Some(c) = track.find_clip_mut(clip_id) {
            c.duration = new_end - c.start;
        }
        sort_track(track);
    }
    sync_link_trim_end(project, track_id, clip_id, delta);
}

/// Mirror a right-trim onto the linked partner.
fn sync_link_trim_end(project: &mut Project, track_id: u64, clip_id: u64, delta: f64) {
    if delta.abs() < 1e-9 {
        return;
    }
    let Some((pt, pc)) = project.find_linked(track_id, clip_id) else {
        return;
    };
    if project.track(pt).map(|t| t.locked).unwrap_or(true) {
        return;
    }
    let Some(p) = project.track(pt).and_then(|t| t.find_clip(pc)).cloned() else {
        return;
    };
    let asset_len = project.asset(p.asset_id).and_then(|a| a.duration());
    let max_end = match asset_len {
        Some(len) => p.start + (len - p.in_point) / p.speed,
        None => p.start + MAX_IMAGE_CLIP_DUR,
    };
    let ne = (p.end() + delta).clamp(p.start + MIN_CLIP_DUR, max_end);
    if let Some(t) = project.track_mut(pt) {
        if let Some(c) = t.find_clip_mut(pc) {
            c.duration = ne - c.start;
        }
        sort_track(t);
    }
}

/// Split the clip under `time` on `track_id` at exactly `time`.
/// Returns the id of the new (right-hand) clip.
///
/// Primitive single-clip split: the right half starts unlinked, the left
/// half keeps the original link. Pair-aware callers (`split_linked`,
/// `split_all_at`) relink the halves afterwards. Fades are redistributed
/// (the right half never re-fades in); keyframes shift; transitions are
/// dropped (they require adjacent clips).
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
    right.link = None;
    // Effects redistribution.
    right.transition_out = None;
    right.fade_in = 0.0; // the fade window belongs to the left half
    right.fade_out = clip.fade_out.min(right.duration);
    right.keyframes = clip
        .keyframes
        .iter()
        .filter(|k| k.t > left_dur)
        .map(|k| Keyframe { t: k.t - left_dur, ..*k })
        .collect();
    let right_id = right.id;
    let track = project.track_mut(track_id)?;
    if let Some(c) = track.find_clip_mut(clip.id) {
        c.duration = left_dur;
        c.transition_out = None;
        c.fade_out = clip.fade_out.min(left_dur);
        c.fade_in = clip.fade_in.min(left_dur);
        c.keyframes = clip
            .keyframes
            .iter()
            .filter(|k| k.t <= left_dur)
            .copied()
            .collect();
    }
    track.insert_clip(right);
    Some(right_id)
}

/// Can the clip be split at `time` (margins respected)?
fn can_split(project: &Project, track_id: u64, clip_id: u64, time: f64) -> bool {
    project
        .find_clip(track_id, clip_id)
        .map(|c| time - c.start >= MIN_CLIP_DUR && c.end() - time >= MIN_CLIP_DUR)
        .unwrap_or(false)
}

fn set_link(project: &mut Project, clip_id: u64, link: Option<u64>) {
    for t in &mut project.tracks {
        if let Some(c) = t.find_clip_mut(clip_id) {
            c.link = link;
            return;
        }
    }
}

fn unlink_pair(project: &mut Project, c1: u64, c2: u64) {
    set_link(project, c1, None);
    set_link(project, c2, None);
}

/// Split a clip — and its linked A/V partner, when both can split — keeping
/// the pairs in sync (left↔left, right↔right). Returns created clip ids.
pub fn split_linked(project: &mut Project, track_id: u64, clip_id: u64, time: f64) -> Vec<u64> {
    let partner = project.find_linked(track_id, clip_id);
    match partner {
        Some((pt, pc)) => {
            if can_split(project, track_id, clip_id, time) && can_split(project, pt, pc, time) {
                let vr = split_at(project, track_id, time);
                let ar = split_at(project, pt, time);
                if let (Some(vr), Some(ar)) = (vr, ar) {
                    set_link(project, vr, Some(ar));
                    set_link(project, ar, Some(vr));
                    return vec![vr, ar];
                }
                // One half failed (should not happen after can_split) —
                // fall through with whatever was created.
                vr.into_iter().chain(ar).collect()
            } else if can_split(project, track_id, clip_id, time) {
                // Partner cannot follow the cut → break the link so the
                // halves stay independently editable.
                unlink_pair(project, clip_id, pc);
                split_at(project, track_id, time).into_iter().collect()
            } else {
                Vec::new()
            }
        }
        None => split_at(project, track_id, time).into_iter().collect(),
    }
}

/// Split whatever clips intersect `time` on every unlocked track.
/// Linked A/V pairs are cut together and stay linked. Returns ids of
/// created clips.
pub fn split_all_at(project: &mut Project, time: f64) -> Vec<u64> {
    // Snapshot the candidates (track, clip) before mutating.
    let mut candidates: Vec<(u64, u64)> = Vec::new();
    for t in &project.tracks {
        if t.locked {
            continue;
        }
        if let Some(c) = t.clip_at(time) {
            candidates.push((t.id, c.id));
        }
    }
    let mut created = Vec::new();
    let mut done: Vec<u64> = Vec::new();
    for (tid, cid) in candidates {
        if done.contains(&cid) {
            continue;
        }
        let partner = project.find_linked(tid, cid);
        let both_can = partner
            .map(|(pt, pc)| can_split(project, pt, pc, time))
            .unwrap_or(false);
        match partner {
            Some((pt, pc)) if both_can && can_split(project, tid, cid, time) => {
                done.push(cid);
                done.push(pc);
                let vr = split_at(project, tid, time);
                let ar = split_at(project, pt, time);
                if let (Some(vr), Some(ar)) = (vr, ar) {
                    set_link(project, vr, Some(ar));
                    set_link(project, ar, Some(vr));
                    created.push(vr);
                    created.push(ar);
                } else {
                    created.extend(vr);
                    created.extend(ar);
                }
            }
            Some((_pt, pc)) => {
                // Only one half can be cut → break the link first.
                done.push(cid);
                done.push(pc);
                unlink_pair(project, cid, pc);
                if let Some(r) = split_at(project, tid, time) {
                    created.push(r);
                }
            }
            None => {
                done.push(cid);
                if let Some(r) = split_at(project, tid, time) {
                    created.push(r);
                }
            }
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

/// Remove a clip and its linked A/V partner (if any).
/// Returns how many clips were removed.
pub fn remove_clip_with_link(project: &mut Project, track_id: u64, clip_id: u64) -> usize {
    let partner = project.find_linked(track_id, clip_id);
    let mut n = 0;
    if remove_clip(project, track_id, clip_id) {
        n += 1;
    }
    if let Some((pt, pc)) = partner {
        if remove_clip(project, pt, pc) {
            n += 1;
        }
    }
    n
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

// ---------------------------------------------------------------------------
// v0.2.0: title clips, transitions, keyframes
// ---------------------------------------------------------------------------

/// Insert a text/title clip on a video track. Returns the clip id.
pub fn add_title_clip(project: &mut Project, track_id: u64, start: f64, text: TextClipData) -> crate::error::Result<u64> {
    const TITLE_DUR: f64 = 4.0;
    {
        let track = project
            .track(track_id)
            .ok_or_else(|| crate::error::VeditError::other("track not found"))?;
        if track.kind != TrackKind::Video {
            return Err(crate::error::VeditError::other("titles go on video tracks"));
        }
        if track.locked {
            return Err(crate::error::VeditError::other("track is locked"));
        }
    }
    let start = {
        let track = project.track(track_id).unwrap();
        clamp_placement(track, None, start.max(0.0), TITLE_DUR)
    };
    let clip = Clip {
        id: project.alloc_id(),
        asset_id: 0,
        start,
        in_point: 0.0,
        duration: TITLE_DUR,
        speed: 1.0,
        volume: 0.0,
        opacity: 1.0,
        muted: false,
        link: None,
        fade_in: 0.0,
        fade_out: 0.0,
        transition_out: None,
        keyframes: Vec::new(),
        text: Some(text),
        grade: None,
    };
    let id = clip.id;
    project.track_mut(track_id).unwrap().insert_clip(clip);
    Ok(id)
}

/// The clip that follows `clip` on the same track (adjacency by start time).
fn next_clip_on_track<'a>(track: &'a crate::project::Track, clip: &Clip) -> Option<&'a Clip> {
    track
        .clips
        .iter()
        .filter(|c| c.id != clip.id && c.start >= clip.end() - 1e-6)
        .min_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal))
}

/// Effective cross-dissolve overlap between `clip` and the clip that follows
/// it on the same track. Returns 0.0 when there is no transition or when the
/// follower has no source material to pull in early.
///
/// The dissolve works by pulling the FOLLOWING clip `d` seconds early (its
/// source read starts at `in_point - d·speed`), so the overlap is clamped by
/// both clip lengths and by the follower's available head material.
pub fn effective_overlap(project: &Project, track_id: u64, clip: &Clip) -> f64 {
    let Some(tr) = clip.transition_out else {
        return 0.0;
    };
    let Some(track) = project.track(track_id) else {
        return 0.0;
    };
    let Some(next) = next_clip_on_track(track, clip) else {
        return 0.0;
    };
    let mut d = tr.duration.max(0.0);
    d = d.min(clip.duration * 0.9).min(next.duration * 0.9);
    if next.in_point.is_finite() {
        let max_pull = next.in_point / next.speed.max(1e-6);
        d = d.min(max_pull);
    }
    if d < MIN_CLIP_DUR {
        0.0
    } else {
        d
    }
}

/// Add (or move) a keyframe at clip-local time `t` with the CURRENT base
/// values for the property that is not explicitly keyed yet. Returns the
/// updated keyframe list. Keeps the list sorted by `t`.
pub fn upsert_keyframe(
    clip: &mut Clip,
    t: f64,
    set_opacity: Option<f64>,
    set_volume: Option<f64>,
) {
    let t = t.clamp(0.0, clip.duration);
    let opacity = set_opacity.unwrap_or_else(|| {
        interp_or(clip, t, |k| k.opacity, clip.opacity)
    });
    let volume = set_volume.unwrap_or_else(|| {
        interp_or(clip, t, |k| k.volume, clip.volume)
    });
    if let Some(k) = clip.keyframes.iter_mut().find(|k| (k.t - t).abs() < 1.0 / 240.0) {
        if set_opacity.is_some() {
            k.opacity = opacity;
        }
        if set_volume.is_some() {
            k.volume = volume;
        }
        return;
    }
    clip.keyframes.push(Keyframe { t, opacity, volume });
    clip.keyframes.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
}

fn interp_or(clip: &Clip, t: f64, pick: impl Fn(&Keyframe) -> f64, base: f64) -> f64 {
    if clip.keyframes.is_empty() {
        return base;
    }
    let curve: crate::project::Curve = clip.keyframes.iter().map(|k| (k.t, pick(k))).collect();
    Clip::eval_curve(&curve, t)
}

/// Remove keyframes within a frame of `t`. Returns true when one was removed.
pub fn remove_keyframe_at(clip: &mut Clip, t: f64) -> bool {
    let before = clip.keyframes.len();
    clip.keyframes.retain(|k| (k.t - t).abs() > 1.0 / 240.0);
    clip.keyframes.len() != before
}
