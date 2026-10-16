//! Engine smoke test: builds synthetic media with ffmpeg, imports them into a
//! project, composes a timeline and renders a real MP4 through the same code
//! path the GUI uses.
//!
//! Run from the VEDIT root:   cargo run -p vedit-core --example smoke_render
//! (requires ffmpeg + ffprobe on PATH)

use std::path::PathBuf;

use vedit_core::export::{
    build_export_command, AudioSettings, ExportSettings, FpsChoice, RateMode, ResChoice,
    VideoSettings,
};
use vedit_core::ffmpeg::FfmpegLocator;
use vedit_core::preview::{decode_frame, snapshot_at};
use vedit_core::project::{Project, TrackKind};
use vedit_core::timeline;

fn main() {
    let tmp = std::env::temp_dir().join("vedit-smoke");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let locator = FfmpegLocator::discover().expect("ffmpeg/ffprobe must be on PATH");
    println!("ffmpeg: {}", locator.ffmpeg().display());

    // ---- 1. create test media (5s test video + 4s audio + image) ----
    let video = tmp.join("test-video.mp4");
    let audio = tmp.join("test-audio.wav");
    let image = tmp.join("test-image.png");
    run(&mut cmd(&locator.ffmpeg(), &[
        "-y", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30:duration=5",
        "-f", "lavfi", "-i", "sine=frequency=440:duration=5",
        "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest",
    ]), &video);
    run(&mut cmd(&locator.ffmpeg(), &[
        "-y", "-f", "lavfi", "-i", "sine=frequency=220:duration=4",
        "-c:a", "pcm_s16le",
    ]), &audio);
    run(&mut cmd(&locator.ffmpeg(), &[
        "-y", "-f", "lavfi", "-i", "smptebars=size=640x360", "-frames:v", "1",
    ]), &image);
    println!("test media created");

    // ---- 2. probe + import ----
    let mut project = Project::new("Smoke Test", 1280, 720, 30.0);
    for path in [&video, &audio, &image] {
        let info = vedit_core::probe::probe(&locator, path).expect("probe failed");
        let id = project.alloc_id();
        let asset = vedit_core::media::make_asset(id, path.clone(), info).unwrap();
        println!(
            "imported {:?} -> {:?} ({}×{})",
            asset.name, asset.kind, asset.info.width.unwrap_or(0), asset.info.height.unwrap_or(0)
        );
        project.assets.push(asset);
    }
    let video_id = project.assets[0].id;
    let audio_id = project.assets[1].id;
    let image_id = project.assets[2].id;

    // ---- 3. build timeline: video on V1, image on V2, audio on A1 ----
    let v1 = project.tracks.iter().find(|t| t.kind == TrackKind::Video && t.name == "V1").unwrap().id;
    let v2 = project.tracks.iter().find(|t| t.kind == TrackKind::Video && t.name == "V2").unwrap().id;
    let a1 = project.tracks.iter().find(|t| t.kind == TrackKind::Audio && t.name == "A1").unwrap().id;

    let _ = timeline::add_asset_clip(&mut project, video_id, v1, 0.0);
    let img_clip = timeline::add_asset_clip(&mut project, image_id, v2, 1.0).expect("add image clip");
    let aud_clip = timeline::add_asset_clip(&mut project, audio_id, a1, 0.5).expect("add audio clip");
    // set opacity on the image clip + trim audio
    if let Some(t) = project.track_mut(v2) {
        if let Some(c) = t.find_clip_mut(img_clip) {
            c.opacity = 0.6;
        }
    }
    timeline::trim_end(&mut project, a1, aud_clip, 3.5);
    // split the video clip at 2.5s
    timeline::split_at(&mut project, v1, 2.5).expect("split failed");

    println!("timeline duration: {:.2}s, clips: {}", project.duration(), project.clip_count());

    // Save the demo project so the GUI can open it:
    //   vedit /tmp/vedit-smoke/demo.vedit.json
    project.save(&tmp.join("demo.vedit.json")).expect("save failed");
    println!("demo project saved: {}", tmp.join("demo.vedit.json").display());

    // ---- 4. preview: decode one composited frame ----
    let snap = snapshot_at(&project, 2.0, 0.5).expect("no layers at t=2.0");
    println!(
        "preview snapshot: {} layer(s), {}×{}",
        snap.layers.len(),
        snap.width,
        snap.height
    );
    let frame = decode_frame(&locator.ffmpeg(), &snap.layers[0], snap.width, snap.height, snap.fps, 2.0)
        .expect("preview frame decode failed");
    let expect = snap.width as usize * snap.height as usize * 4;
    assert_eq!(frame.len(), expect, "frame byte count mismatch");
    println!("preview frame OK ({} bytes RGBA)", frame.len());

    // ---- 5. export H.264 MP4 ----
    let out = tmp.join("export.mp4");
    let settings = ExportSettings {
        output: out.clone(),
        container: "mp4",
        video: VideoSettings {
            codec: "libx264",
            rate: RateMode::Crf { crf: 23 },
            preset: Some("ultrafast"),
            enabled: true,
        },
        audio: AudioSettings {
            codec: "aac",
            bitrate_kbps: 128,
            sample_rate: 48_000,
            enabled: true,
        },
        res: ResChoice::Project,
        fps: FpsChoice::Project,
        extra_args: String::new(),
    };
    let mut plan = build_export_command(&locator, &project, &settings).expect("command build failed");
    let status = plan.cmd.status().expect("ffmpeg spawn failed");
    assert!(status.success(), "export render failed");
    let meta = std::fs::metadata(&out).expect("output missing");
    println!("export OK: {} ({} bytes)", out.display(), meta.len());

    // ---- 6. verify the output with ffprobe ----
    let info = vedit_core::probe::probe(&locator, &out).expect("re-probe failed");
    println!(
        "output check: {}×{} @ {:.2}fps, video={}, audio={}, dur={:?}",
        info.width.unwrap_or(0),
        info.height.unwrap_or(0),
        info.fps.unwrap_or(0.0),
        info.has_video,
        info.has_audio,
        info.duration
    );
    assert!(info.has_video && info.has_audio);
    assert!((info.duration.unwrap_or(0.0) - project.duration()).abs() < 0.5);

    // ---- 7. GIF export (palette path) ----
    let gif_out = tmp.join("export.gif");
    let gif_settings = ExportSettings {
        output: gif_out.clone(),
        container: "gif",
        video: VideoSettings {
            codec: "gif",
            rate: RateMode::Bitrate { kbps: 0 },
            preset: None,
            enabled: true,
        },
        audio: AudioSettings {
            codec: "aac",
            bitrate_kbps: 128,
            sample_rate: 48_000,
            enabled: false,
        },
        res: ResChoice::FitWidth(480),
        fps: FpsChoice::Fixed(12.0),
        extra_args: String::new(),
    };
    let mut gif_plan =
        build_export_command(&locator, &project, &gif_settings).expect("gif command failed");
    let st = gif_plan.cmd.status().expect("gif spawn failed");
    assert!(st.success(), "gif export failed");
    println!("GIF export OK: {} ({} bytes)", gif_out.display(), std::fs::metadata(&gif_out).unwrap().len());

    // ---- 8. WebM VP9 export ----
    let webm_out = tmp.join("export.webm");
    let webm_settings = ExportSettings {
        output: webm_out.clone(),
        container: "webm",
        video: VideoSettings {
            codec: "libvpx-vp9",
            rate: RateMode::Crf { crf: 40 },
            preset: None,
            enabled: true,
        },
        audio: AudioSettings {
            codec: "libopus",
            bitrate_kbps: 96,
            sample_rate: 48_000,
            enabled: true,
        },
        res: ResChoice::Fixed(640, 360),
        fps: FpsChoice::Project,
        extra_args: String::new(),
    };
    let mut webm_plan =
        build_export_command(&locator, &project, &webm_settings).expect("webm command failed");
    let st = webm_plan.cmd.status().expect("webm spawn failed");
    assert!(st.success(), "webm export failed");
    let winfo = vedit_core::probe::probe(&locator, &webm_out).expect("webm re-probe failed");
    assert!(winfo.has_video && winfo.has_audio);
    println!("WebM/VP9 export OK: {} ({} bytes)", webm_out.display(), std::fs::metadata(&webm_out).unwrap().len());

    // ---- 9. audio-only MP3 export ----
    let mp3_out = tmp.join("export.mp3");
    let mp3_settings = ExportSettings {
        output: mp3_out.clone(),
        container: "mp3",
        video: VideoSettings {
            codec: "libx264",
            rate: RateMode::Crf { crf: 23 },
            preset: None,
            enabled: false,
        },
        audio: AudioSettings {
            codec: "libmp3lame",
            bitrate_kbps: 192,
            sample_rate: 48_000,
            enabled: true,
        },
        res: ResChoice::Project,
        fps: FpsChoice::Project,
        extra_args: String::new(),
    };
    let mut mp3_plan =
        build_export_command(&locator, &project, &mp3_settings).expect("mp3 command failed");
    let st = mp3_plan.cmd.status().expect("mp3 spawn failed");
    assert!(st.success(), "mp3 export failed");
    let minfo = vedit_core::probe::probe(&locator, &mp3_out).expect("mp3 re-probe failed");
    assert!(minfo.has_audio && !minfo.has_video);
    println!("MP3 audio export OK: {} ({} bytes)", mp3_out.display(), std::fs::metadata(&mp3_out).unwrap().len());

    println!("\nALL ENGINE CHECKS PASSED ✓ (mp4, gif, webm, mp3, preview, timeline)");
}

fn cmd(program: &std::path::Path, args: &[&str]) -> std::process::Command {
    let mut c = std::process::Command::new(program);
    c.args(args);
    c
}

fn run(c: &mut std::process::Command, expected: &PathBuf) {
    c.arg(expected);
    let out = c.output().expect("spawn failed");
    if !out.status.success() {
        panic!(
            "media creation failed for {}:\n{}",
            expected.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
