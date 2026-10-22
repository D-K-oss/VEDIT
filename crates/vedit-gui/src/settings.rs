//! Persisted app settings (`~/.vedit/settings.json`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::panels::scopes::ScopeKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Background-save the project every N minutes.
    #[serde(default = "default_true")]
    pub autosave_enabled: bool,
    #[serde(default = "default_autosave_minutes")]
    pub autosave_minutes: u32,
    /// New sessions start with snapping enabled.
    #[serde(default = "default_true")]
    pub snap_default: bool,
    /// 0 = Full, 1 = Half, 2 = Quarter.
    #[serde(default = "default_quality")]
    pub preview_quality_default: usize,
    /// Optional folder containing ffmpeg/ffprobe (persisted override).
    #[serde(default)]
    pub ffmpeg_dir: Option<String>,
    /// The two scope slots (persisted so the Color page opens as you left it).
    #[serde(default = "default_scope_slots")]
    pub scope_slots: [ScopeKind; 2],
}

fn default_true() -> bool {
    true
}
fn default_autosave_minutes() -> u32 {
    5
}
fn default_quality() -> usize {
    1
}

fn default_scope_slots() -> [ScopeKind; 2] {
    [ScopeKind::Waveform, ScopeKind::Vectorscope]
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            autosave_enabled: true,
            autosave_minutes: 5,
            snap_default: true,
            preview_quality_default: 1,
            ffmpeg_dir: None,
            scope_slots: default_scope_slots(),
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()?;
        Some(PathBuf::from(home).join(".vedit").join("settings.json"))
    }

    pub fn load() -> Settings {
        let Some(p) = Self::path() else {
            return Settings::default();
        };
        std::fs::read(p)
            .ok()
            .and_then(|d| serde_json::from_slice(&d).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(p) = Self::path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, json);
        }
    }
}
