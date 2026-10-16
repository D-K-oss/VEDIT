//! Locating and running the external ffmpeg/ffprobe tools.
//!
//! VEDIT deliberately shells out to ffmpeg instead of linking libav: this
//! keeps the build pure-Rust on every platform and gives the user access to
//! *every* codec their ffmpeg build supports.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::error::{Result, VeditError};

/// Paths to the two external binaries we depend on.
#[derive(Debug, Clone)]
pub struct FfmpegLocator {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}

impl FfmpegLocator {
    /// Search order:
    /// 1. `VEDIT_FFMPEG_DIR` environment variable (folder containing the binaries)
    /// 2. `<exe dir>/ffmpeg/`   (how the Windows installer lays things out)
    /// 3. `<exe dir>/`
    /// 4. current directory + `./ffmpeg/`
    /// 5. plain `PATH`
    pub fn discover() -> Result<Self> {
        let mut searched: Vec<String> = Vec::new();

        let mut roots: Vec<PathBuf> = Vec::new();
        if let Ok(env_dir) = std::env::var("VEDIT_FFMPEG_DIR") {
            roots.push(PathBuf::from(env_dir));
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                roots.push(dir.join("ffmpeg"));
                roots.push(dir.to_path_buf());
            }
        }
        roots.push(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        roots.push(PathBuf::from("ffmpeg"));

        for root in &roots {
            let ffmpeg = root.join(ffmpeg_exe_name());
            let ffprobe = root.join(ffprobe_exe_name());
            let label = root.display().to_string();
            if ffmpeg.is_file() && ffprobe.is_file() {
                return Ok(Self { ffmpeg, ffprobe });
            }
            searched.push(label);
        }

        // Fall back to PATH — if both resolve we are done.
        let via_path = Self {
            ffmpeg: PathBuf::from(ffmpeg_exe_name()),
            ffprobe: PathBuf::from(ffprobe_exe_name()),
        };
        if which_exists(&via_path.ffmpeg) && which_exists(&via_path.ffprobe) {
            return Ok(via_path);
        }
        searched.push("PATH".to_string());

        Err(VeditError::FfmpegMissing {
            hint: format!("searched: {}", searched.join(", ")),
        })
    }

    pub fn ffmpeg(&self) -> &Path {
        &self.ffmpeg
    }

    pub fn ffprobe(&self) -> &Path {
        &self.ffprobe
    }

    /// ffmpeg version string, for the status bar.
    pub fn version(&self) -> String {
        Command::new(&self.ffmpeg)
            .arg("-version")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.lines().next().map(|l| l.to_string()))
            .unwrap_or_else(|| "unknown version".into())
    }
}

fn ffmpeg_exe_name() -> &'static str {
    if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

fn ffprobe_exe_name() -> &'static str {
    if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    }
}

fn which_exists(program: &Path) -> bool {
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return true;
        }
    }
    false
}

/// Spawn helper: hides the console window on Windows and silences output.
pub fn base_command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// Run a command to completion; on failure include the tail of stderr.
pub fn run_checked(cmd: &mut Command) -> Result<()> {
    let output = cmd.stderr(Stdio::piped()).output()?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let tail: Vec<&str> = stderr.lines().rev().take(12).collect();
    let mut tail: Vec<String> = tail.into_iter().rev().map(|s| s.to_string()).collect();
    tail.push(format!("(exit code: {:?})", output.status.code()));
    Err(VeditError::ToolFailed(tail.join("\n")))
}

/// Kill a child process and reap it (avoids zombies).
pub fn kill_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Format seconds for ffmpeg args (avoids scientific notation).
pub fn secs(v: f64) -> String {
    let s = format!("{v:.3}");
    s
}
