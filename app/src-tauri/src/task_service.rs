//! Tauri commands for the task list, plus the active-task selection.
//!
//! Every change emits `tasks://changed` with the full list so all windows stay in sync
//! (the list is small, so sending it whole is simpler than diffing).

use std::path::Path;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::db;
use crate::error::{Error, Result};
use crate::tasks::{self, Task, TaskKind};
use crate::timer::Phase;
use crate::timer_service::TimerService;

pub const CHANGED_EVENT: &str = "tasks://changed";

struct Inner {
    conn: Connection,
    active: Option<i64>,
}

pub struct TaskService {
    inner: Mutex<Inner>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskState {
    pub tasks: Vec<Task>,
    pub active_id: Option<i64>,
}

impl TaskService {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = db::open(path)?;
        // Start with the first open task selected, as the original did.
        let active = tasks::next_open(&conn, None)?;
        Ok(Self {
            inner: Mutex::new(Inner { conn, active }),
        })
    }

    /// Name of the active task, for the window title.
    pub fn active_name(&self) -> Option<String> {
        let inner = self.inner.lock().unwrap();
        let id = inner.active?;
        tasks::get(&inner.conn, id).ok().map(|t| t.name)
    }

    /// Run a read-only query against the database.
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        f(&self.inner.lock().unwrap().conn)
    }

    /// Write a consistent copy of the database to `path` (used when moving it).
    pub fn backup_to(&self, path: &Path) -> Result<()> {
        let inner = self.inner.lock().unwrap();
        inner
            .conn
            .execute("VACUUM INTO ?1", [path.to_string_lossy()])?;
        Ok(())
    }

    /// Credit a finished pomodoro to the active task and log it.
    pub fn record_pomodoro(&self, app: &AppHandle, started: DateTime<Utc>, ended: DateTime<Utc>) {
        let result = self.mutate(app, |inner| {
            let active = inner.active;
            tasks::record_pomodoro(&mut inner.conn, active, started, ended)
        });
        if let Err(e) = result {
            eprintln!("tasks: failed to record pomodoro: {e}");
        }
    }

    /// Run `f`, then emit and return the new state.
    fn mutate<T>(
        &self,
        app: &AppHandle,
        f: impl FnOnce(&mut Inner) -> Result<T>,
    ) -> Result<TaskState> {
        let mut inner = self.inner.lock().unwrap();
        f(&mut inner)?;
        let state = state_of(&inner)?;
        drop(inner);
        let _ = app.emit(CHANGED_EVENT, &state);
        Ok(state)
    }
}

fn state_of(inner: &Inner) -> Result<TaskState> {
    Ok(TaskState {
        tasks: tasks::list(&inner.conn)?,
        active_id: inner.active,
    })
}

fn pomodoro_running(timer: &TimerService) -> bool {
    timer.phase() == Phase::Pomodoro
}

#[tauri::command]
pub fn tasks_state(svc: State<'_, TaskService>) -> Result<TaskState> {
    state_of(&svc.inner.lock().unwrap())
}

#[tauri::command]
pub fn task_create(
    app: AppHandle,
    svc: State<'_, TaskService>,
    timer: State<'_, TimerService>,
    name: String,
    estimated: Option<i64>,
) -> Result<TaskState> {
    // Select the new task, unless that would switch tasks mid-pomodoro.
    let select = !pomodoro_running(&timer);
    svc.mutate(&app, |inner| {
        let task = tasks::create(&inner.conn, &name, estimated, Utc::now())?;
        if select || inner.active.is_none() {
            inner.active = Some(task.id);
        }
        Ok(())
    })
}

#[tauri::command]
pub fn task_update(
    app: AppHandle,
    svc: State<'_, TaskService>,
    id: i64,
    name: String,
    estimated: Option<i64>,
) -> Result<TaskState> {
    svc.mutate(&app, |inner| {
        tasks::update(&inner.conn, id, &name, estimated).map(drop)
    })
}

#[tauri::command]
pub fn task_select(
    app: AppHandle,
    svc: State<'_, TaskService>,
    timer: State<'_, TimerService>,
    id: i64,
) -> Result<TaskState> {
    if pomodoro_running(&timer) {
        return Err(Error::Invalid(
            "cannot switch tasks while a pomodoro is running".into(),
        ));
    }
    svc.mutate(&app, |inner| {
        if tasks::get(&inner.conn, id)?.done {
            return Err(Error::Invalid("task is already done".into()));
        }
        inner.active = Some(id);
        Ok(())
    })
}

/// Mark a task done or not done. Finishing the active task selects the next open one.
#[tauri::command]
pub fn task_set_done(
    app: AppHandle,
    svc: State<'_, TaskService>,
    id: i64,
    done: bool,
) -> Result<TaskState> {
    svc.mutate(&app, |inner| {
        tasks::set_done(&inner.conn, id, done, Utc::now())?;
        if done && inner.active == Some(id) {
            inner.active = tasks::next_open(&inner.conn, Some(id))?;
        } else if !done && inner.active.is_none() {
            inner.active = Some(id);
        }
        Ok(())
    })
}

#[tauri::command]
pub fn task_delete(app: AppHandle, svc: State<'_, TaskService>, id: i64) -> Result<TaskState> {
    svc.mutate(&app, |inner| {
        let next = tasks::next_open(&inner.conn, Some(id))?;
        tasks::delete(&inner.conn, id)?;
        if inner.active == Some(id) {
            inner.active = next;
        }
        Ok(())
    })
}

#[tauri::command]
pub fn task_reorder(
    app: AppHandle,
    svc: State<'_, TaskService>,
    ids: Vec<i64>,
) -> Result<TaskState> {
    svc.mutate(&app, |inner| tasks::reorder(&mut inner.conn, &ids))
}

#[tauri::command]
pub fn tasks_hide_done(app: AppHandle, svc: State<'_, TaskService>) -> Result<TaskState> {
    svc.mutate(&app, |inner| tasks::hide_done(&inner.conn).map(drop))
}

/// Log an unplanned item or interruption against the active task.
#[tauri::command]
pub fn task_log_disturbance(
    app: AppHandle,
    svc: State<'_, TaskService>,
    kind: TaskKind,
    name: String,
    estimated: Option<i64>,
) -> Result<TaskState> {
    svc.mutate(&app, |inner| {
        let parent = inner
            .active
            .ok_or_else(|| Error::Invalid("no active task".into()))?;
        tasks::log_disturbance(&mut inner.conn, parent, kind, &name, estimated, Utc::now())
            .map(drop)
    })
}
