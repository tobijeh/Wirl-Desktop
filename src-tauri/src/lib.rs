//! Wirl desktop app.

pub mod app;
pub mod audio;
pub mod commands;
pub mod dictation;
pub mod hotkeys;
pub mod pill;
pub mod platform;
pub mod sidecar;
pub mod storage;
pub mod sync_server;
pub mod tray;
pub mod updater;
pub mod windows;

use tauri::RunEvent;
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use wirl_core::diagnostics::{DiagEntry, DiagKind};

fn init_logging(paths: &storage::Paths) {
    use simplelog::*;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.logs.join("wirl.log"));
    let mut loggers: Vec<Box<dyn SharedLogger>> = vec![TermLogger::new(
        LevelFilter::Info,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Never,
    )];
    if let Ok(f) = file {
        // Keep the log small: start fresh when it passes 2 MB.
        if f.metadata().map(|m| m.len() > 2_000_000).unwrap_or(false) {
            let _ = f.set_len(0);
        }
        loggers.push(WriteLogger::new(LevelFilter::Info, Config::default(), f));
    }
    let _ = CombinedLogger::init(loggers);
}

fn load_icons(app: &tauri::AppHandle) -> Option<wirl_core::icons::IconSet> {
    let asset = app.asset_resolver().get("assets/icons/icons.json".to_string())?;
    let text = String::from_utf8(asset.bytes).ok()?;
    match wirl_core::icons::parse_icons_json(&text) {
        Ok(s) => Some(s),
        Err(e) => {
            log::error!("{e}");
            None
        }
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            windows::open_settings(app, None);
        }))
        .plugin(hotkeys::plugin())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::update_settings,
            commands::download_model,
            commands::cancel_download,
            commands::set_api_key,
            commands::clear_api_key,
            commands::dict_add,
            commands::dict_edit,
            commands::dict_delete,
            commands::dict_import,
            commands::dict_export,
            commands::pair_begin,
            commands::pair_cancel,
            commands::unpair,
            commands::hotkey_capture,
            commands::hotkey_key,
            commands::hotkey_check,
            commands::hotkeys_reset,
            commands::reset_defaults,
            commands::check_updates,
            commands::restart_to_update,
            commands::test_dictation,
            commands::reveal_developer_options,
            commands::open_link,
            commands::mark_first_run_done,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let a = app::init(handle.clone());
            init_logging(&a.paths);
            log::info!("Wirl {} starting", env!("CARGO_PKG_VERSION"));
            *a.icons.write() = load_icons(&handle);
            a.sidecar.set_restart_hook(|reason| {
                app::get().log_diag(DiagEntry {
                    error: Some(reason),
                    ..DiagEntry::new(DiagKind::SidecarRestart, "Speech engine restarted")
                });
            });
            pill::create(&handle)?;
            tray::create(&handle)?;
            hotkeys::register_all();
            sync_server::start();
            dictation::start_watchdog();
            updater::start_schedule();
            let s = a.settings();
            // Keep the active model loaded while the app runs.
            if a.model_ready(s.model) {
                commands::ensure_model(s.model);
            }
            if !s.first_run_done {
                if s.start_with_windows {
                    let _ = handle.autolaunch().enable();
                }
                windows::open_settings(&handle, Some("setup"));
            }
            tray::refresh();
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Wirl")
        .run(|_app, ev| {
            // Closing the settings window keeps Wirl running in the tray.
            if let RunEvent::ExitRequested { api, code: None, .. } = ev {
                api.prevent_exit();
            }
        });
}
