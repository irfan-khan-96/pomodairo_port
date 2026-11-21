//! Automatic updates from GitHub Releases.
//!
//! The app checks `latest.json` on the latest release a little after start-up
//! and then every few hours (release builds only, and only while
//! "Check for updates automatically" is on). A found update is announced to the
//! UI, which asks before installing it. Updates are signed: the updater refuses
//! anything not signed with the key whose public half is in `tauri.conf.json`.

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::error::{Error, Result};
use crate::settings_service::SettingsService;
use crate::window_service;

pub const STATUS_EVENT: &str = "update://status";

const FIRST_CHECK: Duration = Duration::from_secs(20);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AvailableUpdate {
    pub version: String,
    pub current: String,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateStatus {
    pub current: String,
    pub available: Option<AvailableUpdate>,
    pub installing: bool,
}

#[derive(Default)]
pub struct UpdaterService {
    pending: Mutex<Option<Update>>,
    installing: Mutex<bool>,
}

impl UpdaterService {
    fn status(&self, app: &AppHandle) -> UpdateStatus {
        UpdateStatus {
            current: app.package_info().version.to_string(),
            available: self
                .pending
                .lock()
                .unwrap()
                .as_ref()
                .map(|u| AvailableUpdate {
                    version: u.version.clone(),
                    current: u.current_version.clone(),
                    notes: u.body.clone().filter(|b| !b.trim().is_empty()),
                }),
            installing: *self.installing.lock().unwrap(),
        }
    }
}

pub fn init(app: &AppHandle) {
    app.manage(UpdaterService::default());
    if cfg!(debug_assertions) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            let enabled = app
                .state::<SettingsService>()
                .current()
                .general
                .check_updates;
            if enabled && let Err(e) = check(&app).await {
                // Offline, no release yet, or no update for this platform: try later.
                eprintln!("updater: {e}");
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

fn publish(app: &AppHandle) {
    let status = app.state::<UpdaterService>().status(app);
    let _ = app.emit(STATUS_EVENT, &status);
}

/// Ask GitHub for a newer version; remembers it and tells the UI if there is one.
async fn check(app: &AppHandle) -> Result<Option<AvailableUpdate>> {
    let update = app
        .updater()
        .map_err(friendly)?
        .check()
        .await
        .map_err(friendly)?;
    *app.state::<UpdaterService>().pending.lock().unwrap() = update;
    publish(app);
    Ok(app.state::<UpdaterService>().status(app).available)
}

/// Turn updater errors into something a person can act on.
fn friendly(e: tauri_plugin_updater::Error) -> Error {
    use tauri_plugin_updater::Error as E;
    let msg = match &e {
        E::Reqwest(_) | E::Network(_) => {
            "Couldn't reach GitHub to check for updates. Check your internet connection.".into()
        }
        E::ReleaseNotFound => "No published release was found to update from.".into(),
        E::TargetNotFound(_) | E::TargetsNotFound(_) => {
            "There's no automatic update for this kind of installation. Download the new version from GitHub.".into()
        }
        E::Minisign(_) | E::SignatureUtf8(_) | E::Base64(_) => {
            "The update's signature didn't check out, so it was not installed.".into()
        }
        other => format!("Update failed: {other}"),
    };
    Error::Invalid(msg)
}

#[tauri::command]
pub fn update_status(app: AppHandle) -> UpdateStatus {
    app.state::<UpdaterService>().status(&app)
}

/// Check now (from the menu). `None` means up to date.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<Option<AvailableUpdate>> {
    check(&app).await
}

/// Download, verify and install the pending update, then restart.
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<()> {
    let service = app.state::<UpdaterService>();
    let update = service
        .pending
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| Error::Invalid("There's no update to install.".into()))?;
    {
        let mut installing = service.installing.lock().unwrap();
        if *installing {
            return Ok(());
        }
        *installing = true;
    }
    publish(&app);
    // Save the window position now: on Windows the installer closes the app.
    window_service::persist(&app);
    let result = update.download_and_install(|_, _| {}, || {}).await;
    *service.installing.lock().unwrap() = false;
    if let Err(e) = result {
        publish(&app);
        return Err(friendly(e));
    }
    app.restart();
}
