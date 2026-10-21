//! Where the database lives. The choice can't be stored in the database itself, so
//! it is kept in a small text file in the app config directory (the original used
//! AIR's encrypted local store for the same reason).

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};

use crate::db::DATABASE_FILE;
use crate::error::{Error, Result};
use crate::task_service::TaskService;

const LOCATION_FILE: &str = "database-location.txt";

#[derive(Debug, Clone, Serialize)]
pub struct DatabaseLocation {
    /// Full path of the database file in use.
    pub path: String,
    /// Folder used when no custom location is set.
    pub default_dir: String,
    /// Set by the `POMODAIRO_DB` debug override; the location can't be changed then.
    pub overridden: bool,
}

fn location_file<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<PathBuf> {
    Ok(app.path().app_config_dir()?.join(LOCATION_FILE))
}

fn default_dir<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<PathBuf> {
    app.path().app_data_dir()
}

fn env_override() -> Option<PathBuf> {
    std::env::var_os("POMODAIRO_DB")
        .filter(|_| cfg!(debug_assertions))
        .map(Into::into)
}

/// The database file to open at startup.
pub fn resolve<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<PathBuf> {
    if let Some(path) = env_override() {
        return Ok(path);
    }
    let custom = std::fs::read_to_string(location_file(app)?)
        .ok()
        .map(|s| PathBuf::from(s.trim()))
        .filter(|dir| dir.is_dir());
    Ok(custom.unwrap_or(default_dir(app)?).join(DATABASE_FILE))
}

#[tauri::command]
pub fn database_location(app: AppHandle) -> Result<DatabaseLocation> {
    let path = resolve(&app).map_err(|e| Error::Invalid(e.to_string()))?;
    Ok(DatabaseLocation {
        path: path.display().to_string(),
        default_dir: default_dir(&app)
            .map_err(|e| Error::Invalid(e.to_string()))?
            .display()
            .to_string(),
        overridden: env_override().is_some(),
    })
}

/// Use the database in `dir` (or the default folder when `None`) from now on, then restart.
///
/// If `dir` already has a Pomodairo database it is used as-is (handy for a synced
/// folder); otherwise the current database is copied there first.
#[tauri::command]
pub fn database_move(app: AppHandle, dir: Option<String>) -> Result<()> {
    if env_override().is_some() {
        return Err(Error::Invalid(
            "the database location is set by POMODAIRO_DB".into(),
        ));
    }
    let to_err = |e: tauri::Error| Error::Invalid(e.to_string());
    let current = resolve(&app).map_err(to_err)?;
    let default = default_dir(&app).map_err(to_err)?;
    let target_dir = match dir.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => default.clone(),
    };
    if !target_dir.is_dir() {
        return Err(Error::Invalid(format!(
            "{} is not a folder",
            target_dir.display()
        )));
    }
    let target = target_dir.join(DATABASE_FILE);
    if same_file(&target, &current) {
        return Ok(());
    }
    if !target.exists() {
        app.state::<TaskService>().backup_to(&target)?;
    }

    let file = location_file(&app).map_err(to_err)?;
    let write = || -> std::io::Result<()> {
        if same_file(&target_dir, &default) {
            match std::fs::remove_file(&file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            }
        } else {
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&file, target_dir.display().to_string())
        }
    };
    write().map_err(|e| Error::Invalid(format!("cannot save database location: {e}")))?;

    crate::window_service::persist(&app);
    app.restart();
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}
