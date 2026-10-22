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

fn install_panic_logger() {
    // Any panic is written to vedit-panic.log next to the executable (plus
    // stderr in debug builds) so crashes can be diagnosed without a console.
    std::panic::set_hook(Box::new(|info| {
        let msg = format!(
            "[{}] panic in {}\n{}\n",
            chrono_like_now(),
            std::thread::current().name().unwrap_or("<unnamed>"),
            info
        );
        eprintln!("VEDIT {msg}");
        let target = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("vedit-panic.log")))
            .unwrap_or_else(|| std::path::PathBuf::from("vedit-panic.log"));
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&target) {
            use std::io::Write;
            let _ = f.write_all(msg.as_bytes());
        }
    }));
}

/// Cheap "YYYY-MM-DD HH:MM:SS" stamp without pulling chrono in.
fn chrono_like_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant's algorithm), good through 2099.
    let z = days as i64 + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn main() -> eframe::Result<()> {
    install_panic_logger();
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
