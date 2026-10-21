//! Drives the timer engine: a background tick loop, Tauri commands, and the side
//! effects of each transition (sounds, window attention, window title, UI events).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tauri::{AppHandle, Emitter, Manager, UserAttentionType};
use tauri_plugin_notification::NotificationExt;

use crate::audio::Audio;
use crate::settings::Settings;
use crate::task_service::TaskService;
use crate::timer::{Phase, TimerConfig, TimerEngine, TimerEvent, TimerSnapshot};

pub const SNAPSHOT_EVENT: &str = "timer://snapshot";
pub const TRANSITION_EVENT: &str = "timer://event";

const TICK_INTERVAL: Duration = Duration::from_millis(200);
const APP_TITLE: &str = "Pomodairo";

pub struct TimerService {
    engine: Mutex<TimerEngine>,
    /// What was last sent out, so the loop only pushes changes.
    published: Mutex<Published>,
    audio: Audio,
    /// Wall-clock start of the running pomodoro, for the pomodoro log.
    pomodoro_started: Mutex<Option<DateTime<Utc>>>,
    /// Show remaining time and task in the window title ("Update taskbar").
    update_title: AtomicBool,
    /// Show a desktop notification when a pomodoro or break ends.
    notify: AtomicBool,
}

#[derive(Default)]
struct Published {
    snapshot: Option<TimerSnapshot>,
    title: String,
    task: Option<String>,
}

impl TimerService {
    pub fn new(settings: &Settings) -> Self {
        Self {
            engine: Mutex::new(TimerEngine::new(scaled(settings.timer_config()))),
            published: Mutex::new(Published::default()),
            audio: Audio::spawn(settings.sound_settings()),
            pomodoro_started: Mutex::new(None),
            update_title: AtomicBool::new(settings.general.update_taskbar),
            notify: AtomicBool::new(settings.general.notifications),
        }
    }

    /// Apply changed settings. New timer lengths take effect from the next phase.
    pub fn apply_settings(&self, settings: &Settings) {
        self.engine
            .lock()
            .unwrap()
            .set_config(scaled(settings.timer_config()));
        self.audio.configure(settings.sound_settings());
        self.update_title
            .store(settings.general.update_taskbar, Ordering::Relaxed);
        self.notify
            .store(settings.general.notifications, Ordering::Relaxed);
    }

    pub fn audio(&self) -> &Audio {
        &self.audio
    }

    pub fn phase(&self) -> Phase {
        self.engine.lock().unwrap().phase()
    }
}

/// Start the background loop that advances the timer. Call once from `setup`.
pub fn spawn_tick_loop(app: AppHandle) {
    thread::Builder::new()
        .name("timer".into())
        .spawn(move || {
            loop {
                let svc = app.state::<TimerService>();
                let now = Instant::now();
                let events = svc.engine.lock().unwrap().tick(now);
                publish(&app, &svc, now, &events);
                thread::sleep(TICK_INTERVAL);
            }
        })
        .expect("failed to spawn timer thread");
}

#[tauri::command]
pub fn timer_snapshot(svc: tauri::State<'_, TimerService>) -> TimerSnapshot {
    svc.engine.lock().unwrap().snapshot(Instant::now())
}

#[tauri::command]
pub fn timer_start(app: AppHandle, svc: tauri::State<'_, TimerService>) -> TimerSnapshot {
    start(&app, &svc)
}

#[tauri::command]
pub fn timer_stop(app: AppHandle, svc: tauri::State<'_, TimerService>) -> TimerSnapshot {
    stop(&app, &svc)
}

/// Start a pomodoro (also used by the tray menu).
pub fn start(app: &AppHandle, svc: &TimerService) -> TimerSnapshot {
    let now = Instant::now();
    let events = svc.engine.lock().unwrap().start(now);
    publish(app, svc, now, &events)
}

/// Stop the running pomodoro or break (also used by the tray menu).
pub fn stop(app: &AppHandle, svc: &TimerService) -> TimerSnapshot {
    let now = Instant::now();
    let events = svc.engine.lock().unwrap().stop();
    publish(app, svc, now, &events)
}

/// Apply side effects for `events`, then push the snapshot to the UI, window
/// title and tray if anything changed. Returns the current snapshot.
fn publish(
    app: &AppHandle,
    svc: &TimerService,
    now: Instant,
    events: &[TimerEvent],
) -> TimerSnapshot {
    for event in events {
        apply_side_effects(app, svc, *event);
        let _ = app.emit(TRANSITION_EVENT, event);
    }

    let snapshot = svc.engine.lock().unwrap().snapshot(now);
    let task_name = app.try_state::<TaskService>().and_then(|t| t.active_name());
    let title = if svc.update_title.load(Ordering::Relaxed) {
        window_title(&snapshot, task_name.as_deref())
    } else {
        APP_TITLE.to_string()
    };

    // Decide what changed under the lock, but make the window/tray calls after
    // releasing it: tray calls wait on the main thread, which may itself be
    // waiting for this lock (e.g. while handling a tray menu click).
    let (snapshot_changed, tray_changed, title_changed) = {
        let mut published = svc.published.lock().unwrap();
        let snapshot_changed = published.snapshot != Some(snapshot);
        let tray_changed = snapshot_changed || published.task != task_name;
        let title_changed = published.title != title;
        published.snapshot = Some(snapshot);
        published.task = task_name.clone();
        published.title = title.clone();
        (snapshot_changed, tray_changed, title_changed)
    };
    if snapshot_changed {
        let _ = app.emit(SNAPSHOT_EVENT, snapshot);
    }
    if tray_changed {
        crate::tray::update(app, &snapshot, task_name.as_deref());
    }
    if title_changed && let Some(window) = app.get_webview_window("main") {
        let _ = window.set_title(&title);
    }
    snapshot
}

fn apply_side_effects(app: &AppHandle, svc: &TimerService, event: TimerEvent) {
    let audio = &svc.audio;
    match event {
        TimerEvent::PomodoroStarted => {
            *svc.pomodoro_started.lock().unwrap() = Some(Utc::now());
            audio.start_ticking();
        }
        TimerEvent::PomodoroStopped => audio.stop_ticking(),
        TimerEvent::PomodoroCompleted { .. } => {
            let ended = Utc::now();
            let started = svc.pomodoro_started.lock().unwrap().take().unwrap_or(ended);
            if let Some(tasks) = app.try_state::<TaskService>() {
                tasks.record_pomodoro(app, started, ended);
            }
            audio.play_alarm();
            demand_attention(app, UserAttentionType::Critical);
            let body = match active_task(app) {
                Some(t) => format!("{t}: time for a break. It starts in a few seconds."),
                None => "Time for a break. It starts in a few seconds.".into(),
            };
            notify(app, svc, "Pomodoro finished", &body);
        }
        TimerEvent::BreakEnded => {
            audio.play_break_end();
            demand_attention(app, UserAttentionType::Informational);
            let next = active_task(app)
                .map(|t| format!(" Next up: {t}."))
                .unwrap_or_default();
            notify(
                app,
                svc,
                "Break over",
                &format!("Ready for the next pomodoro?{next}"),
            );
        }
        TimerEvent::BreakStarted { .. } | TimerEvent::BreakStopped => {}
    }
}

fn active_task(app: &AppHandle) -> Option<String> {
    app.try_state::<TaskService>().and_then(|t| t.active_name())
}

fn notify(app: &AppHandle, svc: &TimerService, title: &str, body: &str) {
    if !svc.notify.load(Ordering::Relaxed) {
        return;
    }
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        eprintln!("notification failed: {e}");
    }
}

/// Bring the window forward and flash the taskbar entry, as the original did on time-out.
fn demand_attention(app: &AppHandle, kind: UserAttentionType) {
    crate::window_service::leave_mini(app);
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.request_user_attention(Some(kind));
}

/// Taskbar title, as in the original: `24' Write docs`, `42'' Write docs` in the
/// last minute, `*break* Write docs`.
fn window_title(s: &TimerSnapshot, task: Option<&str>) -> String {
    let name = task.unwrap_or(APP_TITLE);
    match s.phase {
        Phase::Pomodoro if s.remaining_secs >= 60 => format!("{}' {name}", s.remaining_secs / 60),
        Phase::Pomodoro => format!("{}'' {name}", s.remaining_secs),
        Phase::Break => format!("*break* {name}"),
        Phase::Idle | Phase::TimeOut | Phase::BreakOver => APP_TITLE.to_string(),
    }
}

/// Debug builds honour `POMODAIRO_FAST=1`, which turns every minute into a second
/// so a full cycle can be checked by hand.
fn scaled(config: TimerConfig) -> TimerConfig {
    if cfg!(debug_assertions) && std::env::var("POMODAIRO_FAST").is_ok_and(|v| v == "1") {
        TimerConfig {
            pomodoro: config.pomodoro / 60,
            short_break: config.short_break / 60,
            long_break: config.long_break / 60,
            ..config
        }
    } else {
        config
    }
}
