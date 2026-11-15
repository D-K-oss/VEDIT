//! Subtitle / caption interchange — parse and write `.srt` (SubRip) and
//! `.vtt` (WebVTT), turn entries into burn-in text clips and markers into
//! YouTube-style chapter lists.
//!
//! The GUI imports a subtitle file onto a dedicated top track where the
//! entries become ordinary text clips (so they render live in the preview
//! AND burn into every export through the normal text-overlay path), and the
//! export can write a `.srt` sidecar next to the rendered file.

use crate::project::{Project, TextClipData, TrackKind};
use serde::{Deserialize, Serialize};

/// One timed subtitle entry. Times are timeline seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct SubEntry {
    pub start: f64,
    pub end: f64,
    /// Plain text (markup stripped, lines joined with `\n`).
    pub text: String,
}

// ---------------------------------------------------------------------------
// Timestamp parsing / formatting
// ---------------------------------------------------------------------------

/// Parse `HH:MM:SS,mmm` (SRT) or `HH:MM:SS.mmm` / `MM:SS.mmm` (WebVTT).
pub fn parse_timestamp(s: &str) -> Option<f64> {
    let s = s.trim();
    // Normalize the SRT comma to a dot.
    let s = s.replace(',', ".");
    let mut secs = 0.0f64;
    let parts = s.split(':').collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    // Walk left → right: H × 3600 + M × 60 + S.
    for part in parts {
        let v: f64 = part.parse().ok()?;
        secs = secs * 60.0 + v;
    }
    if secs.is_finite() && secs >= 0.0 {
        Some(secs)
    } else {
        None
    }
}

/// Format seconds as SRT timestamp `HH:MM:SS,mmm`.
pub fn format_srt_time(t: f64) -> String {
    let t = t.max(0.0);
    let total_ms = (t * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let total_s = total_ms / 1000;
    let s = total_s % 60;
    let m = (total_s / 60) % 60;
    let h = total_s / 3600;
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
}

/// Format seconds as a YouTube chapter / display timestamp `MM:SS` or
/// `H:MM:SS` (hours only when non-zero — YouTube accepts both, prefers no
/// leading zero on hours).
pub fn format_display_time(t: f64) -> String {
    let t = t.max(0.0).round() as u64;
    let s = t % 60;
    let m = (t / 60) % 60;
    let h = t / 3600;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

// ---------------------------------------------------------------------------
// Markup helpers
// ---------------------------------------------------------------------------

/// Strip light HTML/markup tags (`<i>`, `<b>`, `<font ...>`, `{\an8}` …)
/// from subtitle text.
pub fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for ch in s.chars() {
        match ch {
            '<' => depth += 1,
            '>' => {
                depth = depth.saturating_sub(1);
            }
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    // ass-style override blocks {\an8}
    out.replace("{\\an8}", "")
}

// ---------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------

/// Parse SubRip `.srt` text.
pub fn parse_srt(src: &str) -> Vec<SubEntry> {
    parse_blocks(&normalize(src), true)
}

/// Parse WebVTT `.vtt` text (header line, NOTE/SOURCE blocks and cue
/// identifiers are skipped; `.` decimal separator expected but `,` accepted).
pub fn parse_vtt(src: &str) -> Vec<SubEntry> {
    let src = normalize(src);
    match src.find("->") {
        // Keep everything from the first cue timing onward; the header and
        // NOTE blocks before it can't contain "->" themselves.
        Some(_) => parse_blocks(&src, false),
        None => Vec::new(),
    }
}

/// CRLF / CR → LF so blank-line block splitting works for Windows files.
fn normalize(src: &str) -> String {
    src.replace("\r\n", "\n").replace('\r', "\n")
}

/// Parse by filename extension (`.vtt` → VTT, everything else → SRT).
pub fn parse_file(file_name: &str, src: &str) -> Vec<SubEntry> {
    if file_name.to_ascii_lowercase().ends_with(".vtt") {
        parse_vtt(src)
    } else {
        parse_srt(src)
    }
}

fn parse_blocks(src: &str, srt: bool) -> Vec<SubEntry> {
    let mut out = Vec::new();
    for block in src.split("\n\n") {
        let mut lines = block.lines().map(str::trim).filter(|l| !l.is_empty());
        // Walk lines until we find the timing line; this transparently skips
        // numeric cue ids (SRT) and identifier lines (VTT) and NOTE bodies.
        while let Some(line) = lines.next() {
            let Some((from, to)) = split_timing(line) else {
                if srt && line.starts_with("NOTE") {
                    break;
                }
                continue;
            };
            let (Some(from), Some(to)) = (parse_timestamp(from), parse_timestamp(to)) else {
                break;
            };
            let mut text = String::new();
            for l in lines {
                if split_timing(l).is_some() {
                    break;
                }
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(l);
            }
            let text = strip_tags(&text);
            if text.trim().is_empty() || to <= from {
                break;
            }
            out.push(SubEntry { start: from, end: to, text });
            break;
        }
    }
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out
}

/// Split `00:00:01,000 --> 00:00:04,000` (also tolerates extra alignment
/// attributes after the end timestamp).
fn split_timing(line: &str) -> Option<(&str, &str)> {
    let idx = line.find("-->")?;
    let (a, b) = line.split_at(idx);
    let b = b.trim_start_matches("-->").trim();
    let end = b
        .split_whitespace()
        .next()
        .unwrap_or(b);
    Some((a.trim(), end))
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// Serialize entries as SubRip.
pub fn format_srt(entries: &[SubEntry]) -> String {
    let mut out = String::new();
    for (i, e) in entries.iter().enumerate() {
        out.push_str(&format!("{}\n", i + 1));
        out.push_str(&format!(
            "{} --> {}\n",
            format_srt_time(e.start),
            format_srt_time(e.end)
        ));
        out.push_str(&e.text);
        out.push_str("\n\n");
    }
    out
}

/// Offset every entry by `delta` seconds (negative moves earlier).
pub fn offset_entries(entries: &mut [SubEntry], delta: f64) {
    for e in entries {
        e.start = (e.start + delta).max(0.0);
        e.end = (e.end + delta).max(e.start + 0.05);
    }
}

// ---------------------------------------------------------------------------
// Text-clip conversion
// ---------------------------------------------------------------------------

/// Text-clip payload for one caption: bold white, bottom-centered.
/// `project_height` scales the font so captions look the same at 720p/1080p/4K.
pub fn caption_text(text: &str, project_height: u32) -> TextClipData {
    let h = project_height.max(1) as f64;
    TextClipData {
        content: text.to_string(),
        size: (h * 0.052).round().clamp(18.0, 220.0),
        color: [255, 255, 255, 255],
        bold: true,
        align: 0.5,
        x: 0.5,
        y: 0.885,
        line_spacing: 1.15,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Caption styles
// ---------------------------------------------------------------------------

/// Named caption looks applied when a subtitle file is imported (and offered
/// as presets for any text clip in the Inspector). Persisted in app settings
/// as the default style for new imports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptionStyle {
    #[default]
    /// Bold white, bottom-centered — the classic look.
    Classic,
    /// White text on a rounded dark pill.
    Pill,
    /// White text with a heavy black outline.
    Outline,
    /// White text with a soft drop shadow.
    Shadowed,
    /// Regular weight, slightly transparent, whisper of shadow.
    Minimal,
}

impl CaptionStyle {
    pub const ALL: &'static [CaptionStyle] = &[
        CaptionStyle::Classic,
        CaptionStyle::Pill,
        CaptionStyle::Outline,
        CaptionStyle::Shadowed,
        CaptionStyle::Minimal,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            CaptionStyle::Classic => "Classic",
            CaptionStyle::Pill => "Pill",
            CaptionStyle::Outline => "Outline",
            CaptionStyle::Shadowed => "Shadow",
            CaptionStyle::Minimal => "Minimal",
        }
    }

    /// Restyle `t` in place. Content, size, position and alignment are kept
    /// (except where the look demands it — e.g. Minimal drops the bold face).
    pub fn apply_to(&self, t: &mut TextClipData, project_height: u32) {
        let h = project_height.max(1) as f64;
        t.outline_width = 0.0;
        t.bg_enabled = false;
        t.shadow = 0.0;
        t.letter_spacing = 0.0;
        t.all_caps = false;
        match self {
            CaptionStyle::Classic => {
                t.bold = true;
                t.color = [255, 255, 255, 255];
            }
            CaptionStyle::Pill => {
                t.bold = true;
                t.color = [255, 255, 255, 255];
                t.bg_enabled = true;
                t.bg_color = [0, 0, 0, 190];
                t.bg_padding = (h * 0.022).round().clamp(10.0, 44.0);
                t.bg_radius = (h * 0.014).round().clamp(6.0, 26.0);
            }
            CaptionStyle::Outline => {
                t.bold = true;
                t.color = [255, 255, 255, 255];
                t.outline_width = (t.size * 0.085).round().clamp(2.0, 18.0);
                t.outline_color = [0, 0, 0, 255];
            }
            CaptionStyle::Shadowed => {
                t.bold = true;
                t.color = [255, 255, 255, 255];
                t.shadow = 0.75;
            }
            CaptionStyle::Minimal => {
                t.bold = false;
                t.color = [232, 230, 227, 235];
                t.shadow = 0.35;
                t.letter_spacing = 0.01;
            }
        }
    }
}

/// Text-clip payload for one caption in the given style.
pub fn caption_text_styled(text: &str, project_height: u32, style: CaptionStyle) -> TextClipData {
    let mut t = caption_text(text, project_height);
    style.apply_to(&mut t, project_height);
    t
}

// ---------------------------------------------------------------------------
// Chapters
// ---------------------------------------------------------------------------

/// Build a YouTube-style chapter list from the timeline markers:
/// `00:00 Intro` lines. YouTube requires the first chapter to start at
/// 00:00 — if no marker sits there (or before 1 s), one named `intro` is
/// prepended automatically.
pub fn youtube_chapters(markers: &[(f64, &str)], fallback_title: &str) -> String {
    let mut pts: Vec<(f64, String)> = markers
        .iter()
        .filter(|(t, _)| *t >= 0.0)
        .map(|(t, l)| (*t, l.trim().to_string()))
        .collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    if pts.first().map(|(t, _)| *t > 0.5).unwrap_or(true) {
        let title = fallback_title.trim();
        pts.insert(0, (0.0, if title.is_empty() { "intro".into() } else { title.to_string() }));
    }
    let mut out = String::new();
    for (t, label) in pts {
        let label = if label.is_empty() { "chapter".to_string() } else { label };
        out.push_str(&format!("{} {}\n", format_display_time(t), label));
    }
    out
}

/// ffmpeg ffmetadata document with a `[CHAPTER]` block per marker.
/// Write this to a temp file and feed it as an extra input (with
/// `-f ffmetadata`) together with `-map_metadata <idx> -map_chapters <idx>`
/// to embed chapters. Times are milliseconds (TIMEBASE=1/1000).
pub fn ffmetadata_chapters(markers: &[(f64, &str)], total: f64) -> String {
    let mut pts: Vec<(f64, String)> = markers
        .iter()
        .filter(|(t, _)| *t >= 0.0)
        .map(|(t, l)| (*t, l.trim().to_string()))
        .collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    pts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6);

    let mut out = String::from(";FFMETADATA1\n");
    for (i, (t, label)) in pts.iter().enumerate() {
        let end = pts.get(i + 1).map(|(nt, _)| *nt).unwrap_or(total.max(t + 1.0));
        let end = end.max(t + 0.05);
        let start_ms = (t * 1000.0).round() as i64;
        let end_ms = (end * 1000.0).round() as i64;
        // Escape ffmetadata special characters (=, ;, #, \ and newline).
        let label = label
            .replace('\\', "\\\\")
            .replace('=', "\\=")
            .replace(';', "\\;")
            .replace('#', "\\#");
        let label = if label.is_empty() { "chapter".to_string() } else { label };
        out.push_str(&format!(
            "[CHAPTER]\nTIMEBASE=1/1000\nSTART={start_ms}\nEND={end_ms}\ntitle={label}\n"
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// Project import
// ---------------------------------------------------------------------------

/// Import subtitle entries as burn-in text clips onto a dedicated
/// "SUBTITLES" video track (created at the very top when missing), styled
/// with `style`. Returns the number of clips added.
pub fn import_into_project(project: &mut Project, entries: &[SubEntry], style: CaptionStyle) -> usize {
    let track_id = ensure_subtitles_track(project);
    let mut added = 0usize;
    for e in entries {
        let clip = crate::project::Clip {
            id: project.alloc_id(),
            asset_id: 0,
            start: e.start,
            duration: (e.end - e.start).max(0.05),
            volume: 0.0, // captions are silent
            text: Some(caption_text_styled(&e.text, project.height, style)),
            ..Default::default()
        };
        if let Some(track) = project.tracks.iter_mut().find(|t| t.id == track_id) {
            track.insert_clip(clip);
            added += 1;
        }
    }
    added
}

/// Find (or create at the very top of the display order) the SUBTITLES track.
pub fn ensure_subtitles_track(project: &mut Project) -> u64 {
    if let Some(t) = project.tracks.iter().find(|t| t.name == "SUBTITLES") {
        return t.id;
    }
    let id = project.alloc_id();
    let track = crate::project::Track {
        id,
        kind: TrackKind::Video,
        name: "SUBTITLES".to_string(),
        muted: false,
        locked: false,
        clips: Vec::new(),
        volume: 1.0,
        pan: 0.0,
        width: 1.0,
        solo: false,
        eq: Default::default(),
        comp: Default::default(),
        ducking: None,
        eq_bass: 0.0,
        eq_mid: 0.0,
        eq_treble: 0.0,
    };
    project.tracks.insert(0, track); // topmost in display order → drawn last
    id
}

/// Collect every text clip of the SUBTITLES track (falling back to ALL text
/// clips when there is no dedicated track) as SRT entries.
pub fn collect_from_project(project: &Project) -> Vec<SubEntry> {
    let dedicated = project.tracks.iter().find(|t| t.name == "SUBTITLES");
    let mut entries = Vec::new();
    for track in project.tracks.iter() {
        // With a dedicated track, only its clips carry captions.
        if dedicated.is_some() && track.name != "SUBTITLES" {
            continue;
        }
        for clip in &track.clips {
            let Some(text) = &clip.text else { continue };
            entries.push(SubEntry {
                start: clip.start,
                end: clip.end(),
                text: text.content.clone(),
            });
        }
    }
    entries.sort_by(|a, b| a.start.total_cmp(&b.start));
    entries
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_SRT: &str = "1\n00:00:01,000 --> 00:00:04,200\nHello world\nsecond line\n\n2\n00:00:05,500 --> 00:00:07,000\n<b>Bold</b> and <i>italic</i>\n\n3\n00:00:08,000 --> 00:00:09,750\n\n4\n00:00:09,000 --> 00:00:08,000\ninverted\n";

    #[test]
    fn srt_parse_and_format_roundtrip() {
        let entries = parse_srt(SAMPLE_SRT);
        assert_eq!(entries.len(), 2, "empty-text + inverted blocks dropped");
        assert_eq!(entries[0].start, 1.0);
        assert!((entries[0].end - 4.2).abs() < 1e-9);
        assert!(entries[0].text.contains("second line"));
        assert_eq!(entries[1].text, "Bold and italic");

        let back = format_srt(&entries);
        let reparsed = parse_srt(&back);
        assert_eq!(reparsed, entries);
        assert!(back.contains("00:00:01,000 --> 00:00:04,200"));
    }

    #[test]
    fn vtt_parse_skips_header_and_notes() {
        let src = "WEBVTT\n\nNOTE this is a comment\nwith two lines\n\nintro\n00:00.500 --> 00:02.000 line one\nline two\n\n00:03:10.250 --> 00:03:12.000 align:left\nlater cue\n";
        let entries = parse_vtt(src);
        assert_eq!(entries.len(), 2);
        assert!((entries[0].start - 0.5).abs() < 1e-9);
        assert!(entries[0].text.contains("line two"));
        assert!((entries[1].start - 190.25).abs() < 1e-9);
        assert_eq!(entries[1].text, "later cue");
    }

    #[test]
    fn timestamps_both_separators_and_short_form() {
        assert!((parse_timestamp("01:02:03,250").unwrap() - 3723.25).abs() < 1e-9);
        assert!((parse_timestamp("01:02:03.25").unwrap() - 3723.25).abs() < 1e-9);
        assert!((parse_timestamp("02:03.5").unwrap() - 123.5).abs() < 1e-9);
        assert!(parse_timestamp("nope").is_none());
        assert!(parse_timestamp("1:2:3:4").is_none());
    }

    #[test]
    fn offset_clamps_and_keeps_length() {
        let mut e = SubEntry { start: 0.2, end: 1.5, text: "x".into() };
        offset_entries(std::slice::from_mut(&mut e), -1.0);
        assert_eq!(e.start, 0.0);
        assert!((e.end - 0.5).abs() < 1e-9, "length shrunk to >= 50 ms floor");
    }

    #[test]
    fn youtube_chapters_starts_at_zero() {
        let markers = vec![(12.0, "second"), (63.0, "minute mark")];
        let text = youtube_chapters(&markers, "My Film");
        assert!(text.starts_with("00:00 My Film\n"));
        assert!(text.contains("00:12 second\n"));
        assert!(text.contains("01:03 minute mark\n"));

        let zeroed = youtube_chapters(&[(0.0, "cold open")], "t");
        assert_eq!(zeroed, "00:00 cold open\n");
    }

    #[test]
    fn ffmetadata_escapes_and_bounds() {
        let doc = ffmetadata_chapters(&[(0.0, "a=b;c#d\\e"), (10.0, "")], 15.0);
        assert!(doc.starts_with(";FFMETADATA1\n"));
        assert!(doc.contains("title=a\\=b\\;c\\#d\\\\e"));
        assert!(doc.contains("[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=10000\n"));
        assert!(doc.contains("END=15000"), "last chapter ends at total");
        assert!(ffmetadata_chapters(&[], 5.0) == ";FFMETADATA1\n");
    }

    #[test]
    fn import_creates_top_track_and_clips() {
        let mut p = crate::project::Project::new("T", 1280, 720, 30.0);
        let v2_before = p.tracks[0].id;
        let entries = vec![
            SubEntry { start: 0.5, end: 2.0, text: "one".into() },
            SubEntry { start: 2.5, end: 4.0, text: "two".into() },
        ];
        let n = import_into_project(&mut p, &entries, CaptionStyle::Classic);
        assert_eq!(n, 2);
        assert_eq!(p.tracks[0].name, "SUBTITLES");
        assert_ne!(p.tracks[0].id, v2_before);
        assert_eq!(p.tracks[0].clips.len(), 2);
        assert!((p.tracks[0].clips[0].duration - 1.5).abs() < 1e-9);
        // Re-import reuses the track.
        let n2 = import_into_project(&mut p, &entries, CaptionStyle::Classic);
        assert_eq!(n2, 2);
        assert_eq!(p.tracks.len(), 5);
        // Collection falls back to the dedicated track only.
        let got = collect_from_project(&p);
        assert_eq!(got.len(), 4);
        assert_eq!(got[0].text, "one");
    }

    #[test]
    fn collect_without_dedicated_track_uses_all_text_clips() {
        let mut p = crate::project::Project::new("T", 1280, 720, 30.0);
        let tid = p.tracks[0].id;
        let clip = crate::project::Clip {
            id: p.alloc_id(),
            start: 1.0,
            duration: 2.0,
            text: Some(caption_text("loose title", 720)),
            ..Default::default()
        };
        p.tracks.iter_mut().find(|t| t.id == tid).unwrap().insert_clip(clip);
        let got = collect_from_project(&p);
        assert_eq!(got.len(), 1);
        assert!((got[0].start - 1.0).abs() < 1e-9);
    }

    #[test]
    fn caption_text_scales_with_project_height() {
        let small = caption_text("hi", 720);
        let big = caption_text("hi", 2160);
        assert!((small.size - 37.0).abs() < 1.5);
        assert!((big.size - 112.0).abs() < 2.0);
        assert_eq!(small.y, big.y);
    }

    #[test]
    fn caption_style_presets_differ_and_roundtrip() {
        let h = 720;
        let classic = caption_text_styled("x", h, CaptionStyle::Classic);
        let pill = caption_text_styled("x", h, CaptionStyle::Pill);
        let outline = caption_text_styled("x", h, CaptionStyle::Outline);
        let shadowed = caption_text_styled("x", h, CaptionStyle::Shadowed);
        let minimal = caption_text_styled("x", h, CaptionStyle::Minimal);

        assert!(!classic.bg_enabled && classic.outline_width == 0.0 && classic.shadow == 0.0);
        assert!(pill.bg_enabled && pill.bg_padding >= 10.0);
        assert!(outline.outline_width >= 2.0 && !outline.bg_enabled);
        assert!(shadowed.shadow > 0.5 && shadowed.outline_width == 0.0);
        assert!(!minimal.bold && minimal.shadow > 0.0 && minimal.color[3] < 255);

        // Pill metrics scale with project height.
        let pill4k = caption_text_styled("x", 2160, CaptionStyle::Pill);
        assert!(pill4k.bg_padding > pill.bg_padding, "pill padding must scale");

        // Presets are exhaustive + persisted by name.
        assert_eq!(CaptionStyle::ALL.len(), 5);
        assert_eq!(serde_json::to_string(&CaptionStyle::Pill).unwrap(), "\"Pill\"");
        assert_eq!(
            serde_json::from_str::<CaptionStyle>("\"Shadowed\"").unwrap(),
            CaptionStyle::Shadowed
        );

        // apply_to is idempotent for its own style (except size-dependent
        // outline which is deterministic anyway).
        let mut again = pill.clone();
        CaptionStyle::Pill.apply_to(&mut again, h);
        assert_eq!(pill, again);
    }
}
