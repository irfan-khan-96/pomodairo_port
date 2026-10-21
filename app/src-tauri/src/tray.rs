//! System tray icon: a live countdown in the tooltip (and next to the icon on
//! macOS / Linux), plus a menu to start/stop the timer, show the window, or quit.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::timer::{BreakKind, Phase, TimerSnapshot};

const TRAY_ID: &str = "main";

pub struct TrayState {
    toggle: MenuItem<Wry>,
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, "toggle", "Start Timer", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "Show / Hide Pomodairo", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&toggle, &show, &PredefinedMenuItem::separator(app)?, &quit],
    )?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Pomodairo")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => toggle_timer(app),
            "show" => toggle_window(app),
            "quit" => {
                crate::window_service::persist(app);
                app.exit(0);
            }
            _ => {}
        })
        // Left click brings the window up (Windows / macOS; Linux tray icons only show the menu).
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    app.manage(TrayState { toggle });
    Ok(())
}

/// Refresh the tray for a new timer state.
pub fn update(app: &AppHandle, snapshot: &TimerSnapshot, task: Option<&str>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let _ = tray.set_tooltip(Some(tooltip(snapshot, task)));
    // Shown beside the icon on macOS and Linux; not supported on Windows.
    let _ = tray.set_title(Some(countdown(snapshot).unwrap_or_default()));
    if let Some(state) = app.try_state::<TrayState>() {
        let _ = state.toggle.set_text(toggle_label(snapshot.phase));
    }
}

fn toggle_timer(app: &AppHandle) {
    let svc = app.state::<crate::timer_service::TimerService>();
    match svc.phase() {
        Phase::Idle | Phase::BreakOver => {
            let has_task = app
                .try_state::<crate::task_service::TaskService>()
                .and_then(|t| t.active_name())
                .is_some();
            if has_task {
                crate::timer_service::start(app, &svc);
            } else {
                // Nothing to work on yet: open the window so a task can be added.
                show_main(app);
            }
        }
        _ => {
            crate::timer_service::stop(app, &svc);
        }
    }
}

fn toggle_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false);
    if visible {
        let _ = window.hide();
    } else {
        show_main(app);
    }
}

/// Show, restore and focus the main window.
pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn toggle_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Idle | Phase::BreakOver => "Start Timer",
        Phase::Pomodoro => "Stop Timer",
        Phase::TimeOut | Phase::Break => "Stop Break",
    }
}

fn mm_ss(secs: u64) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// "12:34" while a pomodoro or break is counting down.
fn countdown(s: &TimerSnapshot) -> Option<String> {
    matches!(s.phase, Phase::Pomodoro | Phase::Break).then(|| mm_ss(s.remaining_secs))
}

pub fn tooltip(s: &TimerSnapshot, task: Option<&str>) -> String {
    let task = task.map(|t| format!(" · {t}")).unwrap_or_default();
    let text = match s.phase {
        Phase::Idle => format!("Pomodairo · ready{task}"),
        Phase::Pomodoro => format!("{} left{task}", mm_ss(s.remaining_secs)),
        Phase::TimeOut => format!("Time's up!{task}"),
        Phase::Break => {
            let kind = match s.break_kind {
                Some(BreakKind::Long) => "Long break",
                _ => "Break",
            };
            format!("{kind} · {} left", mm_ss(s.remaining_secs))
        }
        Phase::BreakOver => "Break over · start the next pomodoro".to_string(),
    };
    // Windows truncates tray tooltips at 127 characters.
    text.chars().take(127).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(phase: Phase, remaining_secs: u64, break_kind: Option<BreakKind>) -> TimerSnapshot {
        TimerSnapshot {
            phase,
            break_kind,
            remaining_secs,
            total_secs: 1500,
            completed: 0,
        }
    }

    #[test]
    fn tooltips() {
        assert_eq!(
            tooltip(&snap(Phase::Idle, 1500, None), None),
            "Pomodairo · ready"
        );
        assert_eq!(
            tooltip(&snap(Phase::Pomodoro, 754, None), Some("Docs")),
            "12:34 left · Docs"
        );
        assert_eq!(
            tooltip(&snap(Phase::Break, 61, Some(BreakKind::Long)), Some("Docs")),
            "Long break · 01:01 left"
        );
        assert_eq!(
            tooltip(&snap(Phase::TimeOut, 0, None), Some("Docs")),
            "Time's up! · Docs"
        );
        let long = "x".repeat(300);
        assert_eq!(
            tooltip(&snap(Phase::Pomodoro, 60, None), Some(&long))
                .chars()
                .count(),
            127
        );
    }

    #[test]
    fn countdown_only_while_running() {
        assert_eq!(
            countdown(&snap(Phase::Pomodoro, 59, None)).as_deref(),
            Some("00:59")
        );
        assert_eq!(countdown(&snap(Phase::Idle, 1500, None)), None);
        assert_eq!(toggle_label(Phase::Break), "Stop Break");
    }
}
