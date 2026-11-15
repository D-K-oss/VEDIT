//! Builds a small test project (JSON) for headless GUI smoke tests.
//! Usage: cargo run -p vedit-core --example make_test_project <out.vedit.json> <dir-with-media>

use std::path::PathBuf;

use vedit_core::ffmpeg::FfmpegLocator;
use vedit_core::project::{Project, TrackKind};
use vedit_core::timeline;

fn main() {
    let out = std::env::args().nth(1).expect("usage: <out.json> <media-dir>");
    let dir = std::env::args().nth(2).expect("usage: <out.json> <media-dir>");
    let locator = FfmpegLocator::discover().expect("ffmpeg on PATH");

    let mut project = Project::new("GUI E2E", 1280, 720, 30.0);
    let mut ids = Vec::new();
    for name in ["clip_a.mp4", "clip_b.mp4"] {
        let path = PathBuf::from(&dir).join(name);
        let info = vedit_core::probe::probe(&locator, &path).expect("probe failed");
        let id = project.alloc_id();
        project.assets.push(vedit_core::media::make_asset(id, path, info).unwrap());
        ids.push(id);
    }

    let v1 = project
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video && t.name == "V1")
        .unwrap()
        .id;
    let a1 = project
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Audio && t.name == "A1")
        .unwrap()
        .id;

    timeline::add_asset_clip(&mut project, ids[0], v1, 0.0).unwrap();
    timeline::add_asset_clip(&mut project, ids[1], v1, 3.0).unwrap();
    // linked audio twin for clip_a
    timeline::add_asset_clip(&mut project, ids[0], a1, 0.0).unwrap();

    // Volume automation on the audio twin (timeline automation-line check):
    // a dip at 0.8 s, a boost at 2.2 s, plus a non-unity base volume.
    if let Some(t) = project.track_mut(a1) {
        if let Some(c) = t.clips.first_mut() {
            c.volume = 0.85;
            c.keyframes = vec![
                vedit_core::project::Keyframe {
                    t: 0.8,
                    opacity: 1.0,
                    volume: 0.35,
                    scale_x: None,
                    pos_x: None,
                    pos_y: None,
                    rotation: None,
                },
                vedit_core::project::Keyframe {
                    t: 2.2,
                    opacity: 1.0,
                    volume: 1.6,
                    scale_x: None,
                    pos_x: None,
                    pos_y: None,
                    rotation: None,
                },
            ];
        }
    }

    // A grade on clip_a so the Color page has something to show.
    if let Some(t) = project.track_mut(v1) {
        if let Some(c) = t.clips.first_mut() {
            c.grade = Some(vedit_core::ColorGrade {
                temperature: 0.15,
                saturation: 1.1,
                ..Default::default()
            });
        }
    }

    // Markers so the ruler flags, overview ticks and the Fairlight index
    // have data to show.
    project.add_marker(0.5, "Intro");
    project.add_marker(3.0, "Beat 1");
    project.add_marker(7.25, "Outro");

    project.save(std::path::Path::new(&out)).unwrap();
    println!("saved {out}");
}
