//! User settings (timing, sounds, general), stored in the `config` table.
//!
//! Window options (always on top, remember position) live in `window_service`,
//! and the database location lives outside the database in `db_location`.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::audio::SoundSettings;
use crate::config::ConfigStore;
use crate::error::{Error, Result};
use crate::timer::TimerConfig;

/// Custom sound files larger than this are rejected (they are loaded into memory).
pub const MAX_SOUND_FILE_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimingSettings {
    pub pomodoro_minutes: u32,
    pub short_break_minutes: u32,
    pub long_break_minutes: u32,
    /// Pomodoros between long breaks.
    pub long_break_interval: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoundPrefs {
    pub enabled: bool,
    /// 0 – 100
    pub volume: u32,
    pub fade_ticking: bool,
    /// Custom alarm sound; `None` uses the built-in one.
    pub alarm_file: Option<String>,
    /// Custom ticking sound; `None` uses the built-in one.
    pub ticking_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneralSettings {
    /// Show the remaining time and task in the window/taskbar title.
    pub update_taskbar: bool,
    /// Clicking a task named like "Fix #123" opens this URL with 123 appended.
    pub url_to_open: String,
    /// Desktop notification when a pomodoro or break ends.
    #[serde(default = "default_true")]
    pub notifications: bool,
    /// Closing the window keeps Pomodairo running in the tray (so devices stay in sync).
    #[serde(default)]
    pub keep_in_tray: bool,
    /// Start Pomodairo (hidden in the tray) when the user signs in.
    #[serde(default)]
    pub launch_at_login: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub timing: TimingSettings,
    pub sound: SoundPrefs,
    pub general: GeneralSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            timing: TimingSettings {
                pomodoro_minutes: 25,
                short_break_minutes: 5,
                long_break_minutes: 15,
                long_break_interval: 4,
            },
            sound: SoundPrefs {
                enabled: true,
                volume: 100,
                fade_ticking: true,
                alarm_file: None,
                ticking_file: None,
            },
            general: GeneralSettings {
                update_taskbar: true,
                url_to_open: String::new(),
                notifications: true,
                keep_in_tray: false,
                launch_at_login: false,
            },
        }
    }
}

mod keys {
    pub const POMODORO: &str = "time.pomodoroLength";
    pub const SHORT_BREAK: &str = "time.shortBreakLength";
    pub const LONG_BREAK: &str = "time.longBreakLength";
    pub const LONG_BREAK_INTERVAL: &str = "time.longBreakInterval";
    pub const SOUND_ENABLED: &str = "sound.enabled";
    pub const VOLUME: &str = "sound.volume";
    pub const FADE_TICKING: &str = "sound.fadeTicking";
    pub const ALARM_FILE: &str = "sound.alarmFile";
    pub const TICKING_FILE: &str = "sound.tickingFile";
    pub const UPDATE_TASKBAR: &str = "window.updateTaskbar";
    pub const URL_TO_OPEN: &str = "general.urlToOpen";
    pub const NOTIFICATIONS: &str = "general.notifications";
    pub const KEEP_IN_TRAY: &str = "window.keepInTray";
    pub const LAUNCH_AT_LOGIN: &str = "general.launchAtLogin";
}

/// Allowed ranges, as in the original's sliders.
pub const POMODORO_RANGE: (u32, u32) = (1, 60);
pub const SHORT_BREAK_RANGE: (u32, u32) = (1, 30);
pub const LONG_BREAK_RANGE: (u32, u32) = (1, 60);
pub const INTERVAL_RANGE: (u32, u32) = (1, 30);

fn in_range(v: u32, (lo, hi): (u32, u32)) -> bool {
    (lo..=hi).contains(&v)
}

impl Settings {
    /// Load from the config table; missing or out-of-range values use defaults.
    pub fn load(cfg: &ConfigStore) -> Self {
        let d = Self::default();
        let num = |key, range, default| {
            cfg.get_parsed(key)
                .filter(|v| in_range(*v, range))
                .unwrap_or(default)
        };
        let path = |key| cfg.get(key).filter(|p: &String| !p.trim().is_empty());
        Self {
            timing: TimingSettings {
                pomodoro_minutes: num(keys::POMODORO, POMODORO_RANGE, d.timing.pomodoro_minutes),
                short_break_minutes: num(
                    keys::SHORT_BREAK,
                    SHORT_BREAK_RANGE,
                    d.timing.short_break_minutes,
                ),
                long_break_minutes: num(
                    keys::LONG_BREAK,
                    LONG_BREAK_RANGE,
                    d.timing.long_break_minutes,
                ),
                long_break_interval: num(
                    keys::LONG_BREAK_INTERVAL,
                    INTERVAL_RANGE,
                    d.timing.long_break_interval,
                ),
            },
            sound: SoundPrefs {
                enabled: cfg
                    .get_parsed(keys::SOUND_ENABLED)
                    .unwrap_or(d.sound.enabled),
                volume: num(keys::VOLUME, (0, 100), d.sound.volume),
                fade_ticking: cfg
                    .get_parsed(keys::FADE_TICKING)
                    .unwrap_or(d.sound.fade_ticking),
                alarm_file: path(keys::ALARM_FILE),
                ticking_file: path(keys::TICKING_FILE),
            },
            general: GeneralSettings {
                update_taskbar: cfg
                    .get_parsed(keys::UPDATE_TASKBAR)
                    .unwrap_or(d.general.update_taskbar),
                url_to_open: cfg.get(keys::URL_TO_OPEN).unwrap_or_default(),
                notifications: cfg
                    .get_parsed(keys::NOTIFICATIONS)
                    .unwrap_or(d.general.notifications),
                keep_in_tray: cfg
                    .get_parsed(keys::KEEP_IN_TRAY)
                    .unwrap_or(d.general.keep_in_tray),
                launch_at_login: cfg
                    .get_parsed(keys::LAUNCH_AT_LOGIN)
                    .unwrap_or(d.general.launch_at_login),
            },
        }
    }

    pub fn save(&self, cfg: &ConfigStore) -> Result<()> {
        let t = &self.timing;
        let s = &self.sound;
        cfg.set_many(&[
            (keys::POMODORO, t.pomodoro_minutes.to_string()),
            (keys::SHORT_BREAK, t.short_break_minutes.to_string()),
            (keys::LONG_BREAK, t.long_break_minutes.to_string()),
            (keys::LONG_BREAK_INTERVAL, t.long_break_interval.to_string()),
            (keys::SOUND_ENABLED, s.enabled.to_string()),
            (keys::VOLUME, s.volume.to_string()),
            (keys::FADE_TICKING, s.fade_ticking.to_string()),
            (keys::ALARM_FILE, s.alarm_file.clone().unwrap_or_default()),
            (
                keys::TICKING_FILE,
                s.ticking_file.clone().unwrap_or_default(),
            ),
            (
                keys::UPDATE_TASKBAR,
                self.general.update_taskbar.to_string(),
            ),
            (keys::URL_TO_OPEN, self.general.url_to_open.clone()),
            (keys::NOTIFICATIONS, self.general.notifications.to_string()),
            (keys::KEEP_IN_TRAY, self.general.keep_in_tray.to_string()),
            (
                keys::LAUNCH_AT_LOGIN,
                self.general.launch_at_login.to_string(),
            ),
        ])
    }

    /// Check and normalise values coming from the UI.
    pub fn validated(mut self) -> Result<Self> {
        let t = &self.timing;
        let checks = [
            (t.pomodoro_minutes, POMODORO_RANGE, "Pomodoro length"),
            (
                t.short_break_minutes,
                SHORT_BREAK_RANGE,
                "Short break length",
            ),
            (t.long_break_minutes, LONG_BREAK_RANGE, "Long break length"),
            (t.long_break_interval, INTERVAL_RANGE, "Long break interval"),
            (self.sound.volume, (0, 100), "Volume"),
        ];
        for (value, (lo, hi), name) in checks {
            if !in_range(value, (lo, hi)) {
                return Err(Error::Invalid(format!(
                    "{name} must be between {lo} and {hi}"
                )));
            }
        }
        for file in [&mut self.sound.alarm_file, &mut self.sound.ticking_file] {
            *file = file
                .take()
                .map(|f| f.trim().to_string())
                .filter(|f| !f.is_empty());
            if let Some(path) = file.as_deref() {
                check_sound_file(Path::new(path))?;
            }
        }
        let url = self.general.url_to_open.trim().to_string();
        if !url.is_empty() && !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(Error::Invalid(
                "URL to open must start with http:// or https://".into(),
            ));
        }
        self.general.url_to_open = url;
        Ok(self)
    }

    pub fn timer_config(&self) -> TimerConfig {
        let min = |m: u32| Duration::from_secs(u64::from(m) * 60);
        TimerConfig {
            pomodoro: min(self.timing.pomodoro_minutes),
            short_break: min(self.timing.short_break_minutes),
            long_break: min(self.timing.long_break_minutes),
            long_break_interval: self.timing.long_break_interval,
        }
    }

    pub fn sound_settings(&self) -> SoundSettings {
        SoundSettings {
            enabled: self.sound.enabled,
            fade_ticking: self.sound.fade_ticking,
            volume: self.sound.volume as f32 / 100.0,
            alarm_file: self.sound.alarm_file.clone().map(Into::into),
            ticking_file: self.sound.ticking_file.clone().map(Into::into),
        }
    }
}

/// A custom sound must be a readable, reasonably small file that rodio can decode.
pub fn check_sound_file(path: &Path) -> Result<()> {
    let meta = std::fs::metadata(path)
        .map_err(|e| Error::Invalid(format!("cannot read sound file {}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(Error::Invalid(format!("{} is not a file", path.display())));
    }
    if meta.len() > MAX_SOUND_FILE_BYTES {
        return Err(Error::Invalid(format!(
            "sound file is too large (max {} MB)",
            MAX_SOUND_FILE_BYTES / 1024 / 1024
        )));
    }
    crate::audio::decode_check(path).map_err(|e| {
        Error::Invalid(format!(
            "{} is not a supported audio file: {e}",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_round_trip() {
        let cfg = ConfigStore::in_memory();
        assert_eq!(Settings::load(&cfg), Settings::default());
        let mut s = Settings::default();
        s.timing.pomodoro_minutes = 50;
        s.sound.volume = 35;
        s.sound.alarm_file = Some("C:\\sounds\\bell.mp3".into());
        s.general.url_to_open = "https://tracker/issue/".into();
        s.save(&cfg).unwrap();
        assert_eq!(Settings::load(&cfg), s);
    }

    #[test]
    fn out_of_range_stored_values_fall_back() {
        let cfg = ConfigStore::in_memory();
        cfg.set_many(&[
            ("time.pomodoroLength", "0".into()),
            ("sound.volume", "250".into()),
        ])
        .unwrap();
        let s = Settings::load(&cfg);
        assert_eq!(s.timing.pomodoro_minutes, 25);
        assert_eq!(s.sound.volume, 100);
    }

    #[test]
    fn validation() {
        let mut s = Settings::default();
        s.timing.short_break_minutes = 31;
        assert!(matches!(s.validated(), Err(Error::Invalid(m)) if m.contains("Short break")));

        let mut s = Settings::default();
        s.general.url_to_open = "javascript:alert(1)".into();
        assert!(s.validated().is_err());

        let mut s = Settings::default();
        s.sound.alarm_file = Some("   ".into());
        s.general.url_to_open = "  https://x/  ".into();
        let s = s.validated().unwrap();
        assert_eq!(s.sound.alarm_file, None, "blank path means built-in sound");
        assert_eq!(s.general.url_to_open, "https://x/");

        let mut s = Settings::default();
        s.sound.ticking_file = Some("Z:\\does\\not\\exist.mp3".into());
        assert!(s.validated().is_err());
    }

    #[test]
    fn rejects_non_audio_file() {
        let path = std::env::temp_dir().join("pomodairo-not-audio.mp3");
        std::fs::write(&path, b"definitely not an mp3").unwrap();
        assert!(check_sound_file(&path).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn timer_config_from_minutes() {
        let mut s = Settings::default();
        s.timing.long_break_minutes = 20;
        let c = s.timer_config();
        assert_eq!(c.pomodoro, Duration::from_secs(25 * 60));
        assert_eq!(c.long_break, Duration::from_secs(20 * 60));
    }
}
