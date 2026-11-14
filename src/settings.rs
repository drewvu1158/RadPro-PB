//! User settings, persisted as TOML in the platform config directory.

use crate::layout::LayoutKind;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Shots per session.
    pub shots: u32,
    /// Seconds on the countdown before each shot.
    pub countdown_seconds: u32,
    /// Which composed layout to produce.
    pub layout: LayoutKind,
    /// Light border (true, like classic booths) or dark.
    pub light_border: bool,
    /// Directory the composed strip and shots are written to.
    pub output_dir: PathBuf,
    /// Camera index passed to the capture backend.
    pub camera_index: u32,
    /// Also save the individual source shots next to the strip.
    pub save_individual_shots: bool,
    /// Force the simulated (no hardware) camera feed.
    pub simulate: bool,
    /// A self-contained copy of the applied SBL layout, if selected.
    pub template_path: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shots: 4,
            countdown_seconds: 3,
            layout: LayoutKind::Strip,
            light_border: true,
            output_dir: default_capture_dir(),
            camera_index: 0,
            save_individual_shots: true,
            simulate: false,
            template_path: None,
        }
    }
}

#[derive(Debug)]
pub enum SettingsError {
    Io(std::io::Error),
    Parse(toml::de::Error),
    Serialize(toml::ser::Error),
    Invalid(String),
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsError::Io(e) => write!(f, "could not write settings: {e}"),
            SettingsError::Parse(e) => write!(f, "could not parse settings: {e}"),
            SettingsError::Serialize(e) => write!(f, "could not encode settings: {e}"),
            SettingsError::Invalid(msg) => write!(f, "invalid settings: {msg}"),
        }
    }
}

impl std::error::Error for SettingsError {}

impl Settings {
    /// Reject nonsense values instead of limping along with them.
    pub fn validate(&self) -> Result<(), SettingsError> {
        if !(1..=8).contains(&self.shots) {
            return Err(SettingsError::Invalid(
                "shots must be between 1 and 8".into(),
            ));
        }
        if !(1..=10).contains(&self.countdown_seconds) {
            return Err(SettingsError::Invalid(
                "countdown_seconds must be between 1 and 10".into(),
            ));
        }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, SettingsError> {
        let text = std::fs::read_to_string(path).map_err(SettingsError::Io)?;
        let settings: Settings = toml::from_str(&text).map_err(SettingsError::Parse)?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        self.validate()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(SettingsError::Io)?;
        }
        let text = toml::to_string_pretty(self).map_err(SettingsError::Serialize)?;
        if let Ok(existing) = std::fs::read_to_string(path) {
            if existing == text {
                return Ok(());
            }
            let backup = path.with_file_name(format!(
                "settings.toml.backup-{}",
                chrono::Local::now().format("%Y%m%d-%H%M%S-%f")
            ));
            std::fs::copy(path, backup).map_err(SettingsError::Io)?;
        }
        // Write to a sibling temp file then rename, so a crash mid-write
        // can't leave a torn settings file behind.
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(SettingsError::Io)?;
        std::fs::rename(&tmp, path).map_err(SettingsError::Io)?;
        Ok(())
    }
}

/// `%APPDATA%/sparkbooth/sparkbooth-rust/config/settings.toml` on Windows.
pub fn default_settings_path() -> PathBuf {
    directories::ProjectDirs::from("com", "sparkbooth", "sparkbooth-rust")
        .map(|dirs| dirs.config_dir().join("settings.toml"))
        .unwrap_or_else(|| PathBuf::from("sparkbooth-rust-settings.toml"))
}

/// `<Pictures>/Sparkbooth-rust` (or home directory as a fallback).
pub fn default_capture_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|dirs| dirs.picture_dir().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Sparkbooth-rust")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_settings() {
        let dir = std::env::temp_dir().join(format!(
            "sparkbooth-rust-test-{}-settings",
            std::process::id()
        ));
        let path = dir.join("settings.toml");
        let mut settings = Settings::default();
        settings.shots = 3;
        settings.light_border = false;
        settings.save(&path).unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.shots, 3);
        assert!(!loaded.light_border);
        assert_eq!(loaded.layout, LayoutKind::Strip);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_values_rejected() {
        let mut settings = Settings::default();
        settings.shots = 0;
        assert!(settings.validate().is_err());
        settings.shots = 9;
        assert!(settings.validate().is_err());
        settings.shots = 4;
        settings.countdown_seconds = 0;
        assert!(settings.validate().is_err());
    }

    #[test]
    fn partial_settings_use_defaults() {
        let settings: Settings = toml::from_str("camera_index = 1\n").unwrap();
        assert_eq!(settings.camera_index, 1);
        assert_eq!(settings.shots, 4);
        assert_eq!(settings.output_dir, default_capture_dir());
        settings.validate().unwrap();
    }

    #[test]
    fn missing_file_loads_default() {
        // Settings::load fails on a missing file; the app falls back to default.
        let path = std::env::temp_dir().join("sparkbooth-rust-no-such-file.toml");
        assert!(Settings::load(&path).is_err());
    }
}
