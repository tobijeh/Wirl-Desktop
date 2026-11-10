//! Dictionary sync through the encrypted mailbox relay (docs/sync-protocol.md).
//!
//! This computer and every phone that scanned its code share one secret,
//! kept in Windows Credential Manager. Sync runs on launch, a moment after a
//! dictionary change, when the Dictionary page asks, and every 5 minutes.

use crate::app::{self, App};
use crate::storage;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::Notify;
use wirl_core::diagnostics::{DiagEntry, DiagKind};
use wirl_core::dictionary::{merge_entries, now_ms};
use wirl_core::relay::{sync_once, HttpRelay, Transport};
use wirl_core::sync::{self, derive, Device, DeviceKind, PairingQr};

/// How often sync runs while Wirl is open.
const INTERVAL: Duration = Duration::from_secs(5 * 60);
/// Wait after a dictionary change, so a burst of edits is one sync.
const DEBOUNCE: Duration = Duration::from_secs(2);

/// What the Dictionary page shows.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    /// A sync group exists (a code was shown at least once).
    pub active: bool,
    pub syncing: bool,
    pub last_ok_ms: Option<i64>,
    /// Short plain message after a failed sync.
    pub error: Option<String>,
    /// The other devices in the group (phones), newest first.
    pub devices: Vec<Device>,
}

static STATUS: Mutex<SyncStatus> = Mutex::new(SyncStatus {
    active: false,
    syncing: false,
    last_ok_ms: None,
    error: None,
    devices: Vec::new(),
});
static WAKE: OnceLock<Notify> = OnceLock::new();
static RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static RELAY: OnceLock<HttpRelay> = OnceLock::new();

fn wake() -> &'static Notify {
    WAKE.get_or_init(Notify::new)
}

/// The relay's base URL. A build can override it with WIRL_RELAY_URL.
pub fn relay_url() -> &'static str {
    match option_env!("WIRL_RELAY_URL") {
        Some(u) if !u.trim().is_empty() => u,
        _ => sync::DEFAULT_RELAY,
    }
}

fn relay() -> &'static HttpRelay {
    RELAY.get_or_init(|| HttpRelay::new(relay_url()))
}

pub fn computer_name() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "Wirl computer".into())
}

pub fn status() -> SyncStatus {
    STATUS.lock().clone()
}

fn me(a: &App) -> Device {
    Device {
        id: a.device.lock().device_id.clone(),
        name: computer_name(),
        kind: DeviceKind::Computer,
        last_sync: 0,
    }
}

/// Starts the background loop. Call once at launch.
pub fn start() {
    STATUS.lock().active = storage::get_sync_secret().is_some();
    tauri::async_runtime::spawn(async {
        loop {
            sync_now().await;
            tokio::select! {
                _ = wake().notified() => tokio::time::sleep(DEBOUNCE).await,
                _ = tokio::time::sleep(INTERVAL) => {}
            }
        }
    });
}

/// Asks for a sync soon (after a dictionary change, or from the UI).
pub fn request() {
    wake().notify_one();
}

/// One sync round. Never runs twice at the same time.
pub async fn sync_now() {
    let _guard = RUNNING.lock().await;
    let a = app::get().clone();
    let Some(secret) = storage::get_sync_secret() else {
        *STATUS.lock() = SyncStatus::default();
        return;
    };
    let Ok(keys) = derive(&secret) else {
        set_error(
            &a,
            "The saved sync code is damaged. Use \"Disconnect all phones\" to make a new one.",
        );
        return;
    };
    {
        let mut s = STATUS.lock();
        s.active = true;
        s.syncing = true;
    }
    a.notify();
    let me = me(&a);
    let local = a.dict.lock().entries.clone();
    let result = sync_once(relay(), &keys, &local, &me, now_ms()).await;
    match result {
        Ok(out) => {
            {
                // Merge again with what's here now, in case the user edited
                // something while the sync was in flight.
                let mut d = a.dict.lock();
                let merged = merge_entries(&d.entries, &out.entries);
                if merged != d.entries {
                    d.entries = merged;
                    drop(d);
                    a.save_dictionary();
                }
            }
            let others: Vec<Device> = out.devices.into_iter().filter(|d| d.id != me.id).collect();
            let new_phones: Vec<String> = {
                let s = STATUS.lock();
                let known: HashSet<&str> = s.devices.iter().map(|d| d.id.as_str()).collect();
                let first_round = s.last_ok_ms.is_none();
                others
                    .iter()
                    .filter(|d| d.kind == DeviceKind::Phone && !known.contains(d.id.as_str()) && !first_round)
                    .map(|d| d.name.clone())
                    .collect()
            };
            let mut sorted = others;
            sorted.sort_by_key(|d| std::cmp::Reverse(d.last_sync));
            {
                let mut s = STATUS.lock();
                s.syncing = false;
                s.error = None;
                s.last_ok_ms = Some(now_ms());
                s.devices = sorted;
            }
            for name in new_phones {
                a.log_diag(DiagEntry::new(DiagKind::Sync, format!("{name} joined phone sync")));
                let _ = tauri::Emitter::emit(&a.handle, "wirl://paired", name);
            }
        }
        Err(e) => {
            log::warn!("sync: {e}");
            a.log_diag(DiagEntry {
                error: Some(e.to_string()),
                ..DiagEntry::new(DiagKind::Sync, "Sync failed")
            });
            set_error(&a, &e.plain());
        }
    }
    a.notify();
}

fn set_error(a: &App, msg: &str) {
    let mut s = STATUS.lock();
    s.syncing = false;
    s.error = Some(msg.to_string());
    drop(s);
    a.notify();
}

/// The pairing QR code (JSON and SVG). Creates the sync group the first time.
pub fn pairing_code() -> Result<(String, String), String> {
    let secret = match storage::get_sync_secret() {
        Some(s) => s,
        None => {
            let s = sync::new_secret();
            storage::set_sync_secret(&s)?;
            STATUS.lock().active = true;
            request();
            s
        }
    };
    let qr = PairingQr {
        v: sync::PROTOCOL_VERSION,
        name: computer_name(),
        relay: relay_url().to_string(),
        secret,
    };
    let json = serde_json::to_string(&qr).map_err(|e| e.to_string())?;
    let code = qrcode::QrCode::new(json.as_bytes()).map_err(|e| e.to_string())?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(240, 240)
        .quiet_zone(true)
        .dark_color(qrcode::render::svg::Color("#111116"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build();
    Ok((json, svg))
}

/// Closes the mailbox (phones learn they were disconnected), forgets the
/// secret and starts a new group with this computer's dictionary.
pub async fn disconnect_all() -> Result<(), String> {
    let _guard = RUNNING.lock().await;
    let a = app::get().clone();
    if let Some(old) = storage::get_sync_secret() {
        if let Ok(keys) = derive(&old) {
            relay()
                .close(&keys)
                .await
                .map_err(|e| format!("Couldn't disconnect: {}", e.plain()))?;
        }
    }
    storage::set_sync_secret(&sync::new_secret())?;
    *STATUS.lock() = SyncStatus {
        active: true,
        ..Default::default()
    };
    a.log_diag(DiagEntry::new(DiagKind::Sync, "Disconnected all phones"));
    a.notify();
    request();
    Ok(())
}
