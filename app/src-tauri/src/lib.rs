mod audio;
mod config;
mod db;
mod db_location;
mod error;
mod net;
mod settings;
mod settings_service;
mod stats;
mod stats_service;
mod sync;
mod sync_service;
mod task_service;
mod tasks;
mod timer;
mod timer_service;
mod tray;
mod tray_icon;
mod updater_service;
mod window_service;

use tauri::Manager;

use config::ConfigStore;
use settings_service::SettingsService;
use task_service::TaskService;
use timer_service::TimerService;
use window_service::WindowService;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();
    // Debug builds with their own database (POMODAIRO_DB) may run side by side,
    // e.g. two "devices" on one machine to test sync.
    let side_by_side = cfg!(debug_assertions) && std::env::var_os("POMODAIRO_DB").is_some();
    if !side_by_side {
        // Must be registered first: a second launch just brings this instance forward.
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main(app);
        }));
    }
    builder
        // Launched at login with --hidden: start in the tray.
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .arg("--hidden")
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let db_path = db_location::resolve(app.handle())?;
            app.manage(TaskService::open(&db_path)?);
            let settings = SettingsService::new(ConfigStore::open(&db_path)?);
            app.manage(TimerService::new(&settings.current()));
            app.manage(settings);
            app.manage(WindowService::new(ConfigStore::open(&db_path)?));
            let hidden = std::env::args().any(|a| a == "--hidden");
            if let Err(e) = window_service::restore(app.handle(), !hidden) {
                // Never leave the (initially hidden) window invisible.
                eprintln!("window: failed to restore preferences: {e}");
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                }
            }
            let launch = app
                .state::<SettingsService>()
                .current()
                .general
                .launch_at_login;
            if let Err(e) = settings_service::apply_launch_at_login(app.handle(), launch) {
                eprintln!("settings: {e}");
            }
            if let Err(e) = sync_service::init(app.handle()) {
                // Sync is optional: the app works without it.
                eprintln!("sync: unavailable: {e}");
            }
            if let Err(e) = tray::create(app.handle()) {
                // Some Linux desktops have no tray; the app works without it.
                eprintln!("tray: unavailable: {e}");
            }
            timer_service::spawn_tick_loop(app.handle().clone());
            updater_service::init(app.handle());
            Ok(())
        })
        .on_window_event(window_service::on_window_event)
        .invoke_handler(tauri::generate_handler![
            timer_service::timer_snapshot,
            timer_service::timer_start,
            timer_service::timer_stop,
            task_service::tasks_state,
            task_service::task_create,
            task_service::task_update,
            task_service::task_select,
            task_service::task_set_done,
            task_service::task_delete,
            task_service::task_reorder,
            task_service::tasks_hide_done,
            task_service::task_log_disturbance,
            window_service::window_prefs,
            window_service::window_set_view,
            window_service::window_toggle_mini,
            window_service::window_set_always_on_top,
            window_service::window_set_remember_position,
            settings_service::settings_get,
            settings_service::settings_save,
            settings_service::sound_preview,
            settings_service::open_settings_window,
            db_location::database_location,
            db_location::database_move,
            stats_service::stats_completed,
            stats_service::stats_disturbances,
            stats_service::stats_per_day,
            stats_service::stats_reality,
            stats_service::stats_hashtags,
            stats_service::open_stats_window,
            sync_service::sync_status,
            sync_service::sync_open_pairing,
            sync_service::sync_cancel_pairing,
            sync_service::sync_discover,
            sync_service::sync_pair_with,
            sync_service::sync_confirm_pairing,
            sync_service::sync_unpair,
            sync_service::sync_now,
            updater_service::update_status,
            updater_service::update_check,
            updater_service::update_install,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
