// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod panels;
mod settings;
mod theme;

use eframe::egui;
use settings::Settings;

fn load_icon() -> egui::IconData {
    let bytes = include_bytes!("../../../assets/icon.png");
    let img = image::load_from_memory(bytes).expect("embedded icon.png").to_rgba8();
    egui::IconData {
        width: img.width(),
        height: img.height(),
        rgba: img.into_raw(),
    }
}

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    // Optional CLI argument: path to a .vedit.json project (enables
    // "Open with VEDIT" file associations in the installers).
    let project_arg = std::env::args().nth(1).map(std::path::PathBuf::from);

    // Locator: settings override (ffmpeg dir) wins over auto-discovery.
    let settings = Settings::load();
    let locator = match settings.ffmpeg_dir.as_deref() {
        Some(dir) if !dir.trim().is_empty() => {
            vedit_core::ffmpeg::FfmpegLocator::from_dir(std::path::Path::new(dir.trim()))
                .or_else(|_| vedit_core::ffmpeg::FfmpegLocator::discover())
                .ok()
        }
        _ => vedit_core::ffmpeg::FfmpegLocator::discover().ok(),
    };
    if locator.is_none() {
        eprintln!(
            "VEDIT: ffmpeg/ffprobe were not found.\n\
             Put ffmpeg + ffprobe in a `ffmpeg/` folder next to vedit.exe,\n\
             set VEDIT_FFMPEG_DIR, or install ffmpeg and add it to PATH."
        );
    }

    let icon = load_icon();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("VEDIT")
            .with_inner_size([1600.0, 900.0])
            .with_min_inner_size([1024.0, 640.0])
            .with_icon(std::sync::Arc::new(icon)),
        ..Default::default()
    };

    eframe::run_native(
        "VEDIT",
        options,
        Box::new(move |cc| Ok(Box::new(app::VeditApp::new(cc, locator, project_arg)))),
    )
}
