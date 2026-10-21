//! Tauri commands for the settings window.

use std::sync::Mutex;

use tauri::{AppHandle, Emitter, State};

use crate::audio::SoundKind;
use crate::config::ConfigStore;
use crate::error::Result;
use crate::settings::{self, Settings};
use crate::timer_service::TimerService;
use crate::window_service::{AuxWindow, open_aux_window};

pub const CHANGED_EVENT: &str = "settings://changed";
pub const TAB_EVENT: &str = "settings://tab";
const SETTINGS_WINDOW: &str = "settings";

pub struct SettingsService {
    config: ConfigStore,
    settings: Mutex<Settings>,
}

impl SettingsService {
    pub fn new(config: ConfigStore) -> Self {
        let settings = Settings::load(&config);
        Self {
            config,
            settings: Mutex::new(settings),
        }
    }

    pub fn current(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }
}

#[tauri::command]
pub fn settings_get(svc: State<'_, SettingsService>) -> Settings {
    svc.current()
}

#[tauri::command]
pub fn settings_save(
    app: AppHandle,
    svc: State<'_, SettingsService>,
    timer: State<'_, TimerService>,
    settings: Settings,
) -> Result<Settings> {
    let settings = settings.validated()?;
    settings.save(&svc.config)?;
    *svc.settings.lock().unwrap() = settings.clone();
    timer.apply_settings(&settings);
    let _ = app.emit(CHANGED_EVENT, &settings);
    Ok(settings)
}

/// Play a sound from the settings window. `file` is a custom sound, or `None` for the built-in one.
#[tauri::command]
pub fn sound_preview(
    timer: State<'_, TimerService>,
    kind: SoundKind,
    file: Option<String>,
    volume: u32,
) -> Result<()> {
    let file = file.map(|f| f.trim().to_string()).filter(|f| !f.is_empty());
    if let Some(path) = &file {
        settings::check_sound_file(path.as_ref())?;
    }
    timer
        .audio()
        .preview(kind, file.map(Into::into), volume.min(100) as f32 / 100.0);
    Ok(())
}

/// Open the settings window on `tab`, or focus it if already open.
///
/// Async on purpose: see `open_aux_window`.
#[tauri::command]
pub async fn open_settings_window(app: AppHandle, tab: Option<String>) -> Result<()> {
    let tab: String = tab
        .unwrap_or_else(|| "general".into())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let (window, created) = open_aux_window(
        &app,
        AuxWindow {
            label: SETTINGS_WINDOW,
            route: format!("settings?tab={tab}"),
            title: "Pomodairo Settings",
            size: (440.0, 400.0),
            resizable: false,
            owned: true,
        },
    )?;
    if !created {
        let _ = window.emit(TAB_EVENT, &tab);
    }
    Ok(())
}
