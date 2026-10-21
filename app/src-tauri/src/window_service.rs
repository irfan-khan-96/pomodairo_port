//! Main window behaviour: the three views (mini / short / full), always-on-top, and
//! remembering position and size between runs.
//!
//! Sizes are kept in logical pixels; the position is kept in physical pixels because
//! that is the coordinate space monitors are reported in.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, LogicalSize, LogicalUnit, Manager, PhysicalPosition, PixelUnit, Runtime,
    State, WebviewWindow, Window, WindowEvent, WindowSizeConstraints,
};

use crate::config::ConfigStore;
use crate::error::{Error, Result};

pub const PREFS_EVENT: &str = "window://prefs";
const MAIN: &str = "main";

pub const MIN_WIDTH: f64 = 320.0;
pub const MINI_HEIGHT: f64 = 52.0;
pub const SHORT_HEIGHT: f64 = 167.0;
pub const DEFAULT_FULL_HEIGHT: f64 = 467.0;
pub const MIN_FULL_HEIGHT: f64 = 260.0;
/// Saved heights above this are treated as corrupt (no screen is this tall).
const MAX_SANE_HEIGHT: f64 = 8192.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    /// Small clock and progress bar only.
    Mini,
    /// Clock and buttons.
    Short,
    /// Clock, buttons and the task list.
    Full,
}

impl ViewMode {
    fn as_str(self) -> &'static str {
        match self {
            ViewMode::Mini => "mini",
            ViewMode::Short => "short",
            ViewMode::Full => "full",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "mini" => Some(ViewMode::Mini),
            "short" => Some(ViewMode::Short),
            "full" => Some(ViewMode::Full),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WindowPrefs {
    pub view: ViewMode,
    /// The view to return to when leaving mini view.
    pub regular_view: ViewMode,
    pub width: f64,
    /// Height of the full view; the user can resize it.
    pub full_height: f64,
    pub position: Option<(i32, i32)>,
    pub always_on_top: bool,
    pub remember_position: bool,
}

impl Default for WindowPrefs {
    fn default() -> Self {
        Self {
            view: ViewMode::Full,
            regular_view: ViewMode::Full,
            width: MIN_WIDTH,
            full_height: DEFAULT_FULL_HEIGHT,
            position: None,
            always_on_top: false,
            remember_position: true,
        }
    }
}

mod keys {
    pub const VIEW: &str = "window.view";
    pub const REGULAR_VIEW: &str = "window.regularView";
    pub const WIDTH: &str = "window.width";
    pub const FULL_HEIGHT: &str = "window.fullHeight";
    pub const X: &str = "window.x";
    pub const Y: &str = "window.y";
    pub const ALWAYS_ON_TOP: &str = "window.alwaysOnTop";
    pub const REMEMBER_POSITION: &str = "window.rememberPosition";
}

impl WindowPrefs {
    pub fn load(cfg: &ConfigStore) -> Self {
        let d = Self::default();
        let view = cfg
            .get(keys::VIEW)
            .and_then(|v| ViewMode::parse(&v))
            .unwrap_or(d.view);
        let regular_view = cfg
            .get(keys::REGULAR_VIEW)
            .and_then(|v| ViewMode::parse(&v))
            .filter(|v| *v != ViewMode::Mini)
            .unwrap_or(d.regular_view);
        let remember_position = cfg
            .get_parsed(keys::REMEMBER_POSITION)
            .unwrap_or(d.remember_position);
        let position = match (cfg.get_parsed(keys::X), cfg.get_parsed(keys::Y)) {
            (Some(x), Some(y)) if remember_position => Some((x, y)),
            _ => None,
        };
        Self {
            view,
            regular_view,
            width: cfg
                .get_parsed(keys::WIDTH)
                .filter(|w: &f64| w.is_finite())
                .unwrap_or(d.width)
                .max(MIN_WIDTH),
            full_height: cfg
                .get_parsed(keys::FULL_HEIGHT)
                .filter(|h: &f64| h.is_finite() && *h <= MAX_SANE_HEIGHT)
                .unwrap_or(d.full_height)
                .max(MIN_FULL_HEIGHT),
            position,
            always_on_top: cfg
                .get_parsed(keys::ALWAYS_ON_TOP)
                .unwrap_or(d.always_on_top),
            remember_position,
        }
    }

    pub fn save(&self, cfg: &ConfigStore) -> Result<()> {
        let mut values = vec![
            (keys::VIEW, self.view.as_str().to_string()),
            (keys::REGULAR_VIEW, self.regular_view.as_str().to_string()),
            (keys::WIDTH, self.width.round().to_string()),
            (keys::FULL_HEIGHT, self.full_height.round().to_string()),
            (keys::ALWAYS_ON_TOP, self.always_on_top.to_string()),
            (keys::REMEMBER_POSITION, self.remember_position.to_string()),
        ];
        if let Some((x, y)) = self.position.filter(|_| self.remember_position) {
            values.push((keys::X, x.to_string()));
            values.push((keys::Y, y.to_string()));
        }
        cfg.set_many(&values)?;
        if !self.remember_position {
            cfg.remove(keys::X)?;
            cfg.remove(keys::Y)?;
        }
        Ok(())
    }

    pub fn height(&self) -> f64 {
        match self.view {
            ViewMode::Mini => MINI_HEIGHT,
            ViewMode::Short => SHORT_HEIGHT,
            ViewMode::Full => self.full_height,
        }
    }

    /// Height limits `(min, max)` for the current view; only the full view is
    /// vertically resizable, and it has no maximum.
    fn height_bounds(&self) -> (f64, Option<f64>) {
        match self.view {
            ViewMode::Full => (MIN_FULL_HEIGHT, None),
            _ => (self.height(), Some(self.height())),
        }
    }

    pub fn set_view(&mut self, view: ViewMode) {
        if view != ViewMode::Mini {
            self.regular_view = view;
        }
        self.view = view;
    }

    pub fn toggle_mini(&mut self) {
        let next = if self.view == ViewMode::Mini {
            self.regular_view
        } else {
            ViewMode::Mini
        };
        self.set_view(next);
    }

    /// Record a user resize (logical size). Height only counts in the full view.
    fn record_size(&mut self, width: f64, height: f64) {
        self.width = width.max(MIN_WIDTH);
        if self.view == ViewMode::Full {
            self.full_height = height.clamp(MIN_FULL_HEIGHT, MAX_SANE_HEIGHT);
        }
    }
}

/// True if the window's title strip would land on one of the monitors
/// (rects are physical `(x, y, width, height)`), so a saved position from a
/// since-disconnected monitor is not restored off-screen.
pub fn position_visible(pos: (i32, i32), width: i32, monitors: &[(i32, i32, u32, u32)]) -> bool {
    // Probe a point a little inside the top edge, where the window is dragged from.
    let probe = (pos.0 + width.min(80) / 2, pos.1 + 10);
    monitors.iter().any(|&(mx, my, mw, mh)| {
        probe.0 >= mx && probe.0 < mx + mw as i32 && probe.1 >= my && probe.1 < my + mh as i32
    })
}

pub struct WindowService {
    config: ConfigStore,
    prefs: Mutex<WindowPrefs>,
}

impl WindowService {
    pub fn new(config: ConfigStore) -> Self {
        let prefs = WindowPrefs::load(&config);
        Self {
            config,
            prefs: Mutex::new(prefs),
        }
    }

    fn save(&self) {
        if let Err(e) = self.prefs.lock().unwrap().save(&self.config) {
            eprintln!("window: failed to save preferences: {e}");
        }
    }
}

/// A secondary window (settings, statistics).
pub struct AuxWindow<'a> {
    pub label: &'a str,
    /// App route, e.g. `settings?tab=timing`.
    pub route: String,
    pub title: &'a str,
    pub size: (f64, f64),
    pub resizable: bool,
    /// Owned by the main window, so it stays above it even when that is always-on-top.
    pub owned: bool,
}

/// Focus the window if it is already open (returning it), otherwise create it.
/// Must be called from an async command: building a window in a synchronous
/// command deadlocks on Windows.
pub fn open_aux_window(app: &AppHandle, spec: AuxWindow) -> Result<(WebviewWindow, bool)> {
    if let Some(window) = app.get_webview_window(spec.label) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok((window, false));
    }
    let to_err = |e: tauri::Error| Error::Invalid(e.to_string());
    let mut builder = tauri::WebviewWindowBuilder::new(
        app,
        spec.label,
        tauri::WebviewUrl::App(spec.route.into()),
    )
    .title(spec.title)
    .inner_size(spec.size.0, spec.size.1)
    .min_inner_size(spec.size.0.min(480.0), spec.size.1.min(320.0))
    .resizable(spec.resizable)
    .maximizable(spec.resizable);
    if spec.owned {
        builder = builder.minimizable(false);
        if let Some(main) = app.get_webview_window(MAIN) {
            builder = builder.parent(&main).map_err(to_err)?;
        }
    }
    Ok((builder.build().map_err(to_err)?, true))
}

/// Save the current window geometry now (e.g. before a restart).
pub fn persist<R: Runtime>(app: &AppHandle<R>) {
    if let Some(svc) = app.try_state::<WindowService>() {
        svc.save();
    }
}

/// Apply saved preferences to the (still hidden) main window, then show it.
pub fn restore<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(MAIN) else {
        return Ok(());
    };
    let svc = app.state::<WindowService>();
    let prefs = svc.prefs.lock().unwrap().clone();

    window.set_always_on_top(prefs.always_on_top)?;

    let monitors: Vec<_> = window
        .available_monitors()?
        .iter()
        .map(|m| {
            (
                m.position().x,
                m.position().y,
                m.size().width,
                m.size().height,
            )
        })
        .collect();
    let width_px = (prefs.width * window.scale_factor()?) as i32;
    match prefs
        .position
        .filter(|&p| position_visible(p, width_px, &monitors))
    {
        Some((x, y)) => window.set_position(PhysicalPosition::new(x, y))?,
        None => window.center()?,
    }
    window.show()?;
    // Resize only once visible: on Windows, resizing the hidden frameless window
    // makes it about 30px too tall.
    apply_size(&window, &prefs)?;
    window.set_focus()?;
    Ok(())
}

fn apply_size<R: Runtime>(window: &WebviewWindow<R>, prefs: &WindowPrefs) -> tauri::Result<()> {
    let logical = |v: f64| PixelUnit::Logical(LogicalUnit::new(v));
    let (min_h, max_h) = prefs.height_bounds();
    // Per-dimension limits, so "no maximum" really is none. (A huge max_size made
    // Windows grow the window to 32767px on the next user resize.)
    // Loosen the limits first so the new size is never rejected by the old ones.
    window.set_size_constraints(WindowSizeConstraints {
        min_width: Some(logical(MIN_WIDTH)),
        min_height: Some(logical(MINI_HEIGHT)),
        ..Default::default()
    })?;
    window.set_size(LogicalSize::new(prefs.width, prefs.height()))?;
    window.set_size_constraints(WindowSizeConstraints {
        min_width: Some(logical(MIN_WIDTH)),
        min_height: Some(logical(min_h)),
        max_width: None,
        max_height: max_h.map(logical),
    })?;
    Ok(())
}

/// Track moves and resizes, and save everything when the window closes.
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if window.label() != MAIN {
        return;
    }
    let app = window.app_handle();
    let Some(svc) = app.try_state::<WindowService>() else {
        return;
    };
    // Minimised windows report bogus geometry (e.g. -32000,-32000 or 0x0 on Windows).
    let minimized = window.is_minimized().unwrap_or(false);
    match event {
        WindowEvent::Moved(pos) if !minimized => {
            svc.prefs.lock().unwrap().position = Some((pos.x, pos.y));
        }
        WindowEvent::Resized(size) if !minimized && size.width > 0 && size.height > 0 => {
            let scale = window.scale_factor().unwrap_or(1.0);
            let logical = size.to_logical::<f64>(scale);
            svc.prefs
                .lock()
                .unwrap()
                .record_size(logical.width, logical.height);
        }
        WindowEvent::CloseRequested { .. } => svc.save(),
        _ => {}
    }
}

/// Change the view, resize the window, persist, and tell the UI.
fn update<R: Runtime>(
    app: &AppHandle<R>,
    change: impl FnOnce(&mut WindowPrefs),
) -> Result<WindowPrefs> {
    let svc = app.state::<WindowService>();
    let prefs = {
        let mut prefs = svc.prefs.lock().unwrap();
        change(&mut prefs);
        prefs.clone()
    };
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = apply_size(&window, &prefs);
        let _ = window.set_always_on_top(prefs.always_on_top);
    }
    svc.save();
    let _ = app.emit(PREFS_EVENT, &prefs);
    Ok(prefs)
}

/// Leave mini view so a time-out is noticed, as the original did.
pub fn leave_mini<R: Runtime>(app: &AppHandle<R>) {
    let in_mini = app
        .try_state::<WindowService>()
        .is_some_and(|svc| svc.prefs.lock().unwrap().view == ViewMode::Mini);
    if in_mini {
        let _ = update(app, |p| p.toggle_mini());
    }
}

#[tauri::command]
pub fn window_prefs(svc: State<'_, WindowService>) -> WindowPrefs {
    svc.prefs.lock().unwrap().clone()
}

#[tauri::command]
pub fn window_set_view(app: AppHandle, view: ViewMode) -> Result<WindowPrefs> {
    update(&app, |p| p.set_view(view))
}

#[tauri::command]
pub fn window_toggle_mini(app: AppHandle) -> Result<WindowPrefs> {
    update(&app, |p| p.toggle_mini())
}

#[tauri::command]
pub fn window_set_always_on_top(app: AppHandle, enabled: bool) -> Result<WindowPrefs> {
    update(&app, |p| p.always_on_top = enabled)
}

#[tauri::command]
pub fn window_set_remember_position(app: AppHandle, enabled: bool) -> Result<WindowPrefs> {
    update(&app, |p| p.remember_position = enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_config_is_empty() {
        let prefs = WindowPrefs::load(&ConfigStore::in_memory());
        assert_eq!(prefs, WindowPrefs::default());
        assert_eq!(prefs.height(), DEFAULT_FULL_HEIGHT);
    }

    #[test]
    fn save_and_load_round_trip() {
        let cfg = ConfigStore::in_memory();
        let prefs = WindowPrefs {
            view: ViewMode::Mini,
            regular_view: ViewMode::Short,
            width: 400.0,
            full_height: 600.0,
            position: Some((-1200, 40)),
            always_on_top: true,
            remember_position: true,
        };
        prefs.save(&cfg).unwrap();
        assert_eq!(WindowPrefs::load(&cfg), prefs);
    }

    #[test]
    fn forgetting_position_clears_it() {
        let cfg = ConfigStore::in_memory();
        let mut prefs = WindowPrefs {
            position: Some((10, 10)),
            ..Default::default()
        };
        prefs.save(&cfg).unwrap();
        prefs.remember_position = false;
        prefs.save(&cfg).unwrap();
        assert_eq!(WindowPrefs::load(&cfg).position, None);
        assert_eq!(cfg.get("window.x"), None);
    }

    #[test]
    fn bad_values_fall_back_to_safe_defaults() {
        let cfg = ConfigStore::in_memory();
        cfg.set_many(&[
            ("window.view", "huge".into()),
            ("window.regularView", "mini".into()),
            ("window.width", "100".into()),
            ("window.fullHeight", "32758".into()),
            ("window.alwaysOnTop", "yes".into()),
        ])
        .unwrap();
        let p = WindowPrefs::load(&cfg);
        assert_eq!(p.view, ViewMode::Full);
        assert_eq!(
            p.regular_view,
            ViewMode::Full,
            "mini is never a regular view"
        );
        assert_eq!(p.width, MIN_WIDTH);
        assert_eq!(p.full_height, DEFAULT_FULL_HEIGHT);
        assert!(!p.always_on_top);
    }

    #[test]
    fn mini_toggles_back_to_last_regular_view() {
        let mut p = WindowPrefs::default();
        p.set_view(ViewMode::Short);
        p.toggle_mini();
        assert_eq!((p.view, p.height()), (ViewMode::Mini, MINI_HEIGHT));
        p.toggle_mini();
        assert_eq!(p.view, ViewMode::Short);
        p.set_view(ViewMode::Full);
        p.toggle_mini();
        p.toggle_mini();
        assert_eq!(p.view, ViewMode::Full);
    }

    #[test]
    fn only_full_view_remembers_height() {
        let mut p = WindowPrefs::default();
        p.record_size(500.0, 700.0);
        assert_eq!((p.width, p.full_height), (500.0, 700.0));
        p.set_view(ViewMode::Short);
        p.record_size(450.0, SHORT_HEIGHT);
        assert_eq!((p.width, p.full_height), (450.0, 700.0));
        assert_eq!(p.height_bounds(), (SHORT_HEIGHT, Some(SHORT_HEIGHT)));
        p.set_view(ViewMode::Full);
        assert_eq!(p.height_bounds(), (MIN_FULL_HEIGHT, None));
        p.set_view(ViewMode::Short);
        p.record_size(10.0, 10.0);
        assert_eq!(p.width, MIN_WIDTH);
    }

    #[test]
    fn position_visibility() {
        let monitors = [(0, 0, 1920, 1080), (1920, 0, 2560, 1440)];
        assert!(position_visible((100, 100), 320, &monitors));
        assert!(
            position_visible((3000, 1200), 320, &monitors),
            "second monitor"
        );
        assert!(
            !position_visible((-500, 100), 320, &monitors),
            "left of all monitors"
        );
        assert!(
            !position_visible((100, 1075), 320, &monitors),
            "title strip below the screen"
        );
        assert!(
            position_visible((-30, 0), 320, &monitors),
            "partly off the left edge is fine"
        );
        assert!(!position_visible((100, 100), 320, &[]));
    }
}
