//! CMX3600 EDL export — the interchange format every NLE, colorist suite
//! and conform tool understands.
//!
//! VEDIT writes a plain (non-drop-frame) EDL with one cut event per media
//! clip: video clips on video tracks become `V` events, audio clips become
//! `A` events, both placed by their record timecodes. Gaps produce no
//! events (conformers place each event by its record-in point), text and
//! adjustment layers are skipped (they have no source media), and speed
//! ramps flatten to their average rate (EDL has no ramp concept — the
//! source-out timecode covers the full source span at the average rate).
//!
//! ```text
//! TITLE: My Project
//! FCM: NON-DROP FRAME
//!
//! 001  MYCLIP   V     C        00:00:00:00 00:00:02:00 00:00:00:00 00:00:02:00
//! * FROM CLIP NAME: myclip.mp4
//! ```

use crate::media::MediaKind;
use crate::project::{Project, TrackKind};

/// The eight label names, matching [`crate::project::MARKER_PALETTE`] order
/// (used in the `* LABEL:` comment).
const LABEL_NAMES: [&str; 8] =
    ["Amber", "Red", "Orange", "Green", "Teal", "Blue", "Purple", "Gray"];

/// Format `seconds` of timeline time as CMX3600 timecode `HH:MM:SS:FF`
/// against the project frame rate. Non-integer rates (29.97) use the
/// rounded base (30) — non-drop-frame timecode, exactly what the `FCM`
/// header declares.
pub fn timecode(seconds: f64, fps: f64) -> String {
    let base = fps.round().max(1.0) as u64;
    let frames = (seconds.max(0.0) * base as f64).round() as u64;
    let f = frames % base;
    let secs = frames / base;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60,
        f
    )
}

/// Tape/reel id for an asset: up to 8 alphanumeric characters of the asset
/// name's stem (extension stripped), uppercased; falls back to `AX`
/// (auxiliary source) when the name has no usable characters.
fn reel_of(name: &str) -> String {
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    let clean: String = stem
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect::<String>()
        .to_ascii_uppercase();
    if clean.is_empty() {
        "AX".to_string()
    } else {
        clean
    }
}

/// One cut event awaiting numbering + chronological sort.
struct Event {
    record_in: f64,
    record_out: f64,
    reel: String,
    channel: &'static str,
    src_in: String,
    src_out: String,
    clip_name: String,
    path: Option<String>,
    label: Option<usize>,
}

/// Build the CMX3600 EDL text for `project` at the project's frame rate.
pub fn format_edl(project: &Project) -> String {
    let fps = project.fps.max(1.0);
    let mut out = String::new();
    out.push_str(&format!("TITLE: {}", project.name));
    out.push_str("\nFCM: NON-DROP FRAME\n");

    // Collect events: video tracks emit V events, audio tracks A events.
    // Tracks are stored display top→bottom; numbering runs bottom-up so
    // the base video track is V1 / the base audio track is A1.
    let mut events: Vec<Event> = Vec::new();
    for track in project.tracks.iter().rev() {
        let channel: &'static str = match track.kind {
            TrackKind::Video => "V",
            TrackKind::Audio => "A",
        };
        for c in &track.clips {
            if c.is_text() || c.is_adjustment() || c.muted || track.muted {
                continue; // no source media / not audible
            }
            let Some(asset) = project.asset(c.asset_id) else {
                continue; // missing media cannot conform
            };
            // Track/asset compatibility: video tracks carry visual assets
            // (video + images), audio tracks carry anything that makes
            // sound — including the linked audio twin of a video asset,
            // which is exactly how VEDIT lays out A/V pairs.
            let kind_ok = !matches!(
                (track.kind, asset.kind, asset.has_audio()),
                (TrackKind::Video, MediaKind::Audio, _) | (TrackKind::Audio, _, false)
            );
            if !kind_ok {
                continue;
            }
            let source_span = c.duration * c.avg_speed();
            events.push(Event {
                record_in: c.start,
                record_out: c.start + c.duration,
                reel: reel_of(&asset.name),
                channel,
                src_in: timecode(c.in_point, fps),
                src_out: timecode(c.in_point + source_span, fps),
                clip_name: asset.name.clone(),
                path: Some(asset.path.display().to_string()),
                label: c.label.map(|l| l as usize),
            });
        }
    }

    if events.is_empty() {
        return out;
    }

    // Chronological order (record-in), then stable by channel so a video
    // event precedes its linked audio twin when both start together.
    events.sort_by(|a, b| {
        a.record_in
            .partial_cmp(&b.record_in)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| channel_rank(a.channel).cmp(&channel_rank(b.channel)))
            .then_with(|| {
                a.record_out
                    .partial_cmp(&b.record_out)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    out.push('\n');
    for (i, e) in events.iter().enumerate() {
        out.push_str(&render_event(e, i + 1, fps));
        out.push('\n');
    }
    out
}

fn channel_rank(c: &str) -> u8 {
    match c {
        "V" => 0,
        _ => 1,
    }
}

fn render_event(e: &Event, num: usize, fps: f64) -> String {
    let mut s = format!(
        "{:03}  {:<8} {:<5} {:<8} {} {} {} {}",
        num % 1000,
        e.reel,
        e.channel,
        "C",
        e.src_in,
        e.src_out,
        timecode(e.record_in, fps),
        timecode(e.record_out, fps)
    );
    s.push_str(&format!("\n* FROM CLIP NAME: {}", e.clip_name));
    if let Some(p) = &e.path {
        s.push_str(&format!("\n* SOURCE FILE: {p}"));
    }
    if let Some(l) = e.label {
        s.push_str(&format!(
            "\n* LABEL: {}",
            LABEL_NAMES[l % LABEL_NAMES.len()]
        ));
    }
    s
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Project;
    use crate::timeline;

    /// tiny project: one 2 s video clip on V1 (bottom video track) with a
    /// linked audio twin on A1.
    fn tiny_project() -> Project {
        let mut p = Project::new("EDL Test", 640, 360, 30.0);
        let (video_track, audio_track) = (p.tracks[1].id, p.tracks[2].id);
        let asset_id = p.next_id;
        let asset = crate::media::MediaAsset {
            id: asset_id,
            path: "/media/red clip.mp4".into(),
            name: "red clip.mp4".into(),
            kind: crate::media::MediaKind::Video,
            info: crate::media::MediaInfo {
                duration: Some(4.0),
                has_video: true,
                has_audio: true,
                ..Default::default()
            },
            proxy: None,
        };
        p.next_id += 1;
        p.assets.push(asset);
        timeline::add_asset_clip(&mut p, asset_id, video_track, 0.0).expect("video clip");
        timeline::add_asset_clip(&mut p, asset_id, audio_track, 0.0).expect("audio twin");
        p
    }

    #[test]
    fn timecode_formats_and_uses_rounded_base() {
        assert_eq!(timecode(0.0, 30.0), "00:00:00:00");
        assert_eq!(timecode(1.0, 30.0), "00:00:01:00");
        assert_eq!(timecode(3661.5, 30.0), "01:01:01:15");
        // 23.976 → base 24; 1 s = 24 frames.
        assert_eq!(timecode(1.0, 23.976), "00:00:01:00");
        // Negative clamps to zero.
        assert_eq!(timecode(-2.0, 30.0), "00:00:00:00");
    }

    #[test]
    fn edl_header_and_cut_events_match_cmx3600() {
        let p = tiny_project();
        let edl = format_edl(&p);
        assert!(edl.starts_with("TITLE: EDL Test\nFCM: NON-DROP FRAME\n"));
        // Video event: source 00:00:00:00–00:00:02:00, record the same.
        assert!(
            edl.contains("001  REDCLIP  V     C        00:00:00:00 00:00:04:00 00:00:00:00 00:00:04:00"),
            "video event line missing:\n{edl}"
        );
        assert!(edl.contains("* FROM CLIP NAME: red clip.mp4"), "{edl}");
        assert!(edl.contains("* SOURCE FILE: /media/red clip.mp4"), "{edl}");
        // Audio twin emits its own A event, numbered after the video one.
        assert!(
            edl.contains("002  REDCLIP  A     C        00:00:00:00 00:00:04:00 00:00:00:00 00:00:04:00"),
            "audio event line missing:\n{edl}"
        );
    }

    #[test]
    fn edl_skips_gaps_mutes_text_and_orders_chronologically() {
        let mut p = tiny_project();
        let (video_track, audio_track) = (p.tracks[1].id, p.tracks[2].id);
        // Second video asset starting at 3 s (1 s gap) + a title + a muted clip.
        let asset2_id = p.next_id;
        let asset2 = crate::media::MediaAsset {
            id: asset2_id,
            path: "/media/blue.mp4".into(),
            name: "blue.mp4".into(),
            kind: crate::media::MediaKind::Video,
            info: crate::media::MediaInfo {
                duration: Some(3.0),
                has_video: true,
                has_audio: true,
                ..Default::default()
            },
            proxy: None,
        };
        p.next_id += 1;
        p.assets.push(asset2);
        let c2 = timeline::add_asset_clip(&mut p, asset2_id, video_track, 3.0).expect("blue clip");
        // Mute it → no event.
        p.track_mut(video_track).unwrap().find_clip_mut(c2).unwrap().muted = true;
        // Title on the video track → skipped.
        timeline::add_title_clip(
            &mut p,
            video_track,
            6.0,
            crate::project::TextClipData {
                content: "Hi".into(),
                ..Default::default()
            },
        )
        .expect("title");
        // Audio clip starting at 5 s → event AFTER the blue one's slot but
        // blue is muted, so audio is the second numbered event.
        let first_asset = p.assets[0].id;
        timeline::add_asset_clip(&mut p, first_asset, audio_track, 5.0).expect("audio at 5s");

        let edl = format_edl(&p);
        assert!(edl.contains("001  REDCLIP"), "{edl}");
        assert!(!edl.contains("BLUE"), "muted clip must be skipped:\n{edl}");
        // The title clip has no media → no event for it (only the header
        // line may mention TITLE).
        assert_eq!(edl.matches("TITLE").count(), 1, "header TITLE only:\n{edl}");
        assert!(edl.contains("* FROM CLIP NAME: red clip.mp4"), "{edl}");
        // tiny_project's audio twin at 0 s is event 002; the 5 s audio
        // clip lands as event 003, chronologically after it.
        assert!(edl.contains("003  REDCLIP  A     C        00:00:00:00 00:00:04:00 00:00:05:00 00:00:09:00"), "{edl}");
    }

    #[test]
    fn edl_carries_labels_and_flattens_ramps_to_avg_rate() {
        let mut p = tiny_project();
        let video_track = p.tracks[1].id;
        // Label the video clip Red (index 1) — the linked twin inherits it.
        let vid = p.tracks[1].clips[0].id;
        timeline::set_clip_label(&mut p, video_track, vid, Some(1));
        // 2× speed: source span = 4 s for a 2 s clip.
        p.track_mut(video_track).unwrap().clips[0].speed = 2.0;
        let edl = format_edl(&p);
        assert!(edl.contains("* LABEL: Red"), "{edl}");
        assert!(
            edl.contains("001  REDCLIP  V     C        00:00:00:00 00:00:08:00 00:00:00:00 00:00:04:00"),
            "source out must cover the full 8 s source span (2× ramp):\n{edl}"
        );
    }

    #[test]
    fn edl_empty_project_has_header_only() {
        let p = Project::new("Empty", 640, 360, 30.0);
        let edl = format_edl(&p);
        assert_eq!(edl, "TITLE: Empty\nFCM: NON-DROP FRAME\n");
    }

    #[test]
    fn reel_names_are_sanitized_and_capped() {
        assert_eq!(reel_of("my clip!! final.mp4"), "MYCLIPFI");
        assert_eq!(reel_of("###"), "AX");
        assert_eq!(reel_of(""), "AX");
    }
}
