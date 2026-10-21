//! Tauri commands for the statistics window.

use chrono::Local;
use tauri::{AppHandle, State};

use crate::error::Result;
use crate::stats::{self, CompletedTask, DateRange, DayCount, Disturbance, Hashtags, WeekFactor};
use crate::task_service::TaskService;
use crate::window_service::{AuxWindow, open_aux_window};

#[tauri::command]
pub fn stats_completed(
    svc: State<'_, TaskService>,
    range: DateRange,
    filter: String,
) -> Result<Vec<CompletedTask>> {
    svc.read(|conn| stats::completed_tasks(conn, range, &filter, &Local))
}

#[tauri::command]
pub fn stats_disturbances(
    svc: State<'_, TaskService>,
    range: DateRange,
    filter: String,
) -> Result<Vec<Disturbance>> {
    svc.read(|conn| stats::disturbances(conn, range, &filter, &Local))
}

#[tauri::command]
pub fn stats_per_day(svc: State<'_, TaskService>, range: DateRange) -> Result<Vec<DayCount>> {
    svc.read(|conn| stats::per_day(conn, range, &Local))
}

#[tauri::command]
pub fn stats_reality(svc: State<'_, TaskService>) -> Result<Vec<WeekFactor>> {
    svc.read(|conn| stats::reality_by_week(conn, &Local))
}

#[tauri::command]
pub fn stats_hashtags(svc: State<'_, TaskService>) -> Result<Hashtags> {
    svc.read(stats::hashtags)
}

/// Async on purpose: see `open_aux_window`.
#[tauri::command]
pub async fn open_stats_window(app: AppHandle) -> Result<()> {
    open_aux_window(
        &app,
        AuxWindow {
            label: "stats",
            route: "stats".into(),
            title: "Pomodairo Statistics",
            size: (640.0, 460.0),
            resizable: true,
            owned: false,
        },
    )?;
    Ok(())
}
