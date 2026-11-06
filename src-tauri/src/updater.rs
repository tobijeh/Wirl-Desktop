//! Updates from this repo's GitHub Releases, signature-verified by the
//! Tauri updater. Checks on launch and daily, downloads in the background,
//! shows "Update ready" in the tray, installs on restart.

use crate::app;
use crate::tray;
use parking_lot::Mutex;
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_updater::{Update, UpdaterExt};
use wirl_core::dictionary::now_ms;

struct Ready {
    update: Update,
    bytes: Vec<u8>,
}

static READY: Mutex<Option<Ready>> = Mutex::new(None);

fn set_status(f: impl FnOnce(&mut crate::app::UpdateStatus)) {
    if let Some(a) = app::try_get() {
        f(&mut a.update.lock());
        a.notify();
    }
}

async fn check(app: &AppHandle) -> Result<Option<String>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let version = update.version.clone();
    if READY
        .lock()
        .as_ref()
        .map(|r| r.update.version == version)
        .unwrap_or(false)
    {
        return Ok(Some(version));
    }
    // Downloads and verifies the signature against the bundled public key.
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    *READY.lock() = Some(Ready { update, bytes });
    Ok(Some(version))
}

/// `manual` = user asked; show the outcome on the About page.
pub fn check_now(manual: bool) {
    let Some(a) = app::try_get() else { return };
    let app = a.handle.clone();
    tauri::async_runtime::spawn(async move {
        set_status(|s| {
            s.checking = true;
            s.message = None;
        });
        let r = check(&app).await;
        set_status(|s| {
            s.checking = false;
            s.last_checked_ms = Some(now_ms());
            match &r {
                Ok(Some(v)) => {
                    s.ready_version = Some(v.clone());
                    s.message = Some(format!("Version {v} is ready. Restart to update."));
                }
                Ok(None) => s.message = Some("You're up to date.".into()),
                Err(e) => {
                    log::warn!("update check: {e}");
                    if manual {
                        s.message = Some("Couldn't check for updates. Try again later.".into());
                    }
                }
            }
        });
        tray::refresh();
    });
}

/// Check shortly after launch, then daily.
pub fn start_schedule() {
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(Duration::from_secs(15)).await;
        loop {
            check_now(false);
            tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        }
    });
}

fn install_ready() -> bool {
    let ready = READY.lock().take();
    if let Some(r) = ready {
        if let Err(e) = r.update.install(&r.bytes) {
            log::error!("install update: {e}");
            return false;
        }
        return true;
    }
    false
}

pub fn restart_to_update() {
    let Some(a) = app::try_get() else { return };
    let app = a.handle.clone();
    tauri::async_runtime::spawn(async move {
        a.sidecar.shutdown().await;
        // On Windows the installer takes over and the app exits.
        if install_ready() {
            app.restart();
        }
    });
}

/// Quit, installing a downloaded update on the way out.
pub fn quit_installing_if_ready(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(a) = app::try_get() {
            a.sidecar.shutdown().await;
        }
        install_ready();
        app.exit(0);
    });
}
