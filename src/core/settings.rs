//! Persisted settings: stored in ~/.oxide/settings.json. Create file with defaults if missing.
//!
//! Design: **Single source of truth**. `PersistedSettings` is the only authority for view_mode and
//! show_hidden. AppState syncs from it via `sync_from_persisted_settings()` at startup and
//! whenever a setting changes, so panels (and their file lists) always match before render.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Settings stored on disk. Defaults: Two columns, show hidden on, autosave off.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSettings {
    /// When true, save left/right panel current dirs on navigation (and on exit); restore on start.
    #[serde(default)]
    pub autosave: bool,
    /// When true (default), after returning from shell (Ctrl+O) sync active panel to shell's cwd. When false, use old flow (panel stays as before).
    #[serde(default = "default_true")]
    pub sync_panel_to_shell_cwd: bool,
    /// When true, after running a command from panels, show a short top-line countdown over shell output, then restore panels.
    #[serde(default = "default_true")]
    pub auto_reopen_panels_after_command: bool,
    /// Seconds after the command finishes before returning to the panel TUI (countdown on the main terminal buffer).
    #[serde(default = "default_auto_reopen_panels_delay_secs")]
    pub auto_reopen_panels_after_command_delay_secs: u64,
    /// Saved current directory for left panel. None or invalid => use home on start.
    #[serde(default)]
    pub left_cwd: Option<String>,
    /// Saved current directory for right panel.
    #[serde(default)]
    pub right_cwd: Option<String>,
    /// Left panel view: "one" = SingleColumn, "two" = DoubleColumn (Ctrl+T).
    #[serde(default = "default_view")]
    pub left_view: String,
    #[serde(default = "default_view")]
    pub right_view: String,
    /// Show hidden files (Ctrl+H). Per-panel in UI; on by default.
    #[serde(default = "default_true")]
    pub left_show_hidden: bool,
    #[serde(default = "default_true")]
    pub right_show_hidden: bool,
    /// File/folder sort mode per panel: name_asc, name_desc, size_asc, size_desc, mtime_asc, mtime_desc.
    #[serde(default = "default_sort")]
    pub left_sort: String,
    #[serde(default = "default_sort")]
    pub right_sort: String,
    /// When true (default), directories appear before files; when false, unified sort by the chosen key.
    #[serde(default = "default_true")]
    pub left_dirs_first: bool,
    #[serde(default = "default_true")]
    pub right_dirs_first: bool,
    /// Active panel index when autosave last ran: 0 = left, 1 = right. Restored on start if autosave was on.
    #[serde(default)]
    pub active_panel: u8,
    /// File name matching for Find (Ctrl+F) and +/− pattern selection: "wildcard" (*, ?) or "regex".
    #[serde(default = "default_file_pattern_mode")]
    pub file_pattern_mode: String,
}

fn default_file_pattern_mode() -> String {
    "wildcard".to_string()
}

fn default_view() -> String {
    "two".to_string()
}

fn default_true() -> bool {
    true
}

fn default_auto_reopen_panels_delay_secs() -> u64 {
    2
}

fn default_sort() -> String {
    "name_asc".to_string()
}

impl Default for PersistedSettings {
    fn default() -> Self {
        Self {
            autosave: false,
            sync_panel_to_shell_cwd: true,
            auto_reopen_panels_after_command: true,
            auto_reopen_panels_after_command_delay_secs: default_auto_reopen_panels_delay_secs(),
            left_cwd: None,
            right_cwd: None,
            left_view: "two".to_string(),
            right_view: "two".to_string(),
            left_show_hidden: true,
            right_show_hidden: true,
            left_sort: "name_asc".to_string(),
            right_sort: "name_asc".to_string(),
            left_dirs_first: true,
            right_dirs_first: true,
            active_panel: 0,
            file_pattern_mode: default_file_pattern_mode(),
        }
    }
}

impl PersistedSettings {
    /// True when Find file and +/− use regular expressions; false for shell-style wildcards.
    pub fn file_pattern_uses_regex(&self) -> bool {
        self.file_pattern_mode.as_str() == "regex"
    }
}

/// Directory for config: ~/.oxide
pub fn config_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".oxide"))
}

/// Path to settings file: ~/.oxide/settings.json
pub fn settings_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("settings.json"))
}

/// Ensure ~/.oxide exists. Returns the config dir path or None if home is missing.
pub fn ensure_config_dir() -> Option<PathBuf> {
    let dir = config_dir()?;
    let _ = fs::create_dir_all(&dir);
    Some(dir)
}

/// Load settings from ~/.oxide/settings.json. If file is missing or invalid, returns defaults.
pub fn load() -> PersistedSettings {
    let path = match settings_path() {
        Some(p) => p,
        None => return PersistedSettings::default(),
    };
    let Ok(data) = fs::read_to_string(&path) else {
        return PersistedSettings::default();
    };
    serde_json::from_str(&data).unwrap_or_default()
}

/// Save settings to ~/.oxide/settings.json. Creates .oxide and file if they don't exist.
pub fn save(settings: &PersistedSettings) -> std::io::Result<()> {
    let Some(dir) = ensure_config_dir() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "home dir not found",
        ));
    };
    let path = dir.join("settings.json");
    let data = serde_json::to_string_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    fs::write(path, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_roundtrip() {
        let s = PersistedSettings::default();
        let json = serde_json::to_string(&s).unwrap();
        let _: PersistedSettings = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn panel_settings_persisted_in_json() {
        let s = PersistedSettings::default();
        let json = serde_json::to_string(&s).unwrap();
        // Ensure all panel-related settings are written to settings.json
        assert!(json.contains("\"left_sort\""));
        assert!(json.contains("\"right_sort\""));
        assert!(json.contains("\"left_dirs_first\""));
        assert!(json.contains("\"right_dirs_first\""));
        assert!(json.contains("\"auto_reopen_panels_after_command\""));
        assert!(json.contains("\"auto_reopen_panels_after_command_delay_secs\""));
        assert!(json.contains("\"file_pattern_mode\""));
    }
}
