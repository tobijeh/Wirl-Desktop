//! Shared application state.

use crate::sidecar::Sidecar;
use crate::storage::{self, DeviceInfo, Paths};
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use tauri::{AppHandle, Emitter, Manager};
use wirl_core::claude::ClaudeClient;
use wirl_core::diagnostics::{DiagEntry, DiagKind, Diagnostics};
use wirl_core::dictionary::Dictionary;
use wirl_core::hotkey::{Conflict, DictateTrigger};
use wirl_core::icons::IconSet;
use wirl_core::insert::UndoTracker;
use wirl_core::models;
use wirl_core::settings::{ModelId, Settings};
use wirl_core::watchdog::Watchdog;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Idle,
    Recording,
    Processing,
    CleaningSelection,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadState {
    pub done: u64,
    pub total: u64,
    pub error: Option<String>,
    pub active: bool,
    /// Downloaded and being unpacked (the Cantonese model).
    pub unpacking: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub checking: bool,
    pub ready_version: Option<String>,
    pub last_checked_ms: Option<i64>,
    pub message: Option<String>,
}

pub struct App {
    pub handle: AppHandle,
    pub paths: Paths,
    pub settings: RwLock<Settings>,
    pub dict: Mutex<Dictionary>,
    pub device: Mutex<DeviceInfo>,
    pub diag: Mutex<Diagnostics>,
    pub claude: ClaudeClient,
    pub http: reqwest::Client,
    pub sidecar: Arc<Sidecar>,
    pub icons: RwLock<Option<IconSet>>,
    pub phase: Mutex<Phase>,
    /// Bumped whenever a session ends or is reset; stale tasks check it.
    pub session: AtomicU64,
    pub trigger: Mutex<DictateTrigger>,
    pub undo: Mutex<UndoTracker>,
    pub watchdog: Mutex<Watchdog>,
    pub downloads: Mutex<HashMap<ModelId, DownloadState>>,
    pub download_cancel: Mutex<HashMap<ModelId, Arc<AtomicBool>>>,
    pub update: Mutex<UpdateStatus>,
    pub hotkey_conflicts: Mutex<Vec<(wirl_core::hotkey::Action, Conflict)>>,
    pub capturing_hotkey: AtomicBool,
    pub last_error: Mutex<Option<String>>,
    pub control: Mutex<Option<tokio::sync::mpsc::UnboundedSender<crate::dictation::Control>>>,
    /// The user's API key, cached from the credential store.
    pub api_key: RwLock<Option<String>>,
}

static APP: OnceLock<Arc<App>> = OnceLock::new();

pub fn get() -> &'static Arc<App> {
    APP.get().expect("app initialised")
}

pub fn try_get() -> Option<&'static Arc<App>> {
    APP.get()
}

pub fn init(handle: AppHandle) -> Arc<App> {
    let data = handle
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("Wirl"));
    let paths = Paths::new(data);
    let settings = storage::load_settings(&paths);
    let mut dict = storage::load_dictionary(&paths);
    dict.purge_tombstones(wirl_core::dictionary::now_ms());
    let device = storage::load_device(&paths);
    let hold = settings.hold_to_talk;
    let app = Arc::new(App {
        handle,
        settings: RwLock::new(settings),
        dict: Mutex::new(dict),
        device: Mutex::new(device),
        diag: Mutex::new(Diagnostics::default()),
        claude: ClaudeClient::new(),
        http: reqwest::Client::builder()
            .user_agent(concat!("Wirl/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("http client"),
        sidecar: Sidecar::new(),
        icons: RwLock::new(None),
        phase: Mutex::new(Phase::Idle),
        session: AtomicU64::new(0),
        trigger: Mutex::new(DictateTrigger::new(hold)),
        undo: Mutex::new(UndoTracker::default()),
        watchdog: Mutex::new(Watchdog::default()),
        downloads: Mutex::new(HashMap::new()),
        download_cancel: Mutex::new(HashMap::new()),
        update: Mutex::new(UpdateStatus::default()),
        hotkey_conflicts: Mutex::new(vec![]),
        capturing_hotkey: AtomicBool::new(false),
        last_error: Mutex::new(None),
        control: Mutex::new(None),
        api_key: RwLock::new(storage::get_api_key()),
        paths,
    });
    let _ = APP.set(app.clone());
    app
}

impl App {
    pub fn settings(&self) -> Settings {
        self.settings.read().clone()
    }

    pub fn save_settings(&self) {
        let s = self.settings();
        storage::save_settings(&self.paths, &s);
    }

    pub fn save_dictionary(&self) {
        let d = self.dict.lock().clone();
        storage::save_dictionary(&self.paths, &d);
    }

    pub fn save_device(&self) {
        let d = self.device.lock().clone();
        storage::save_device(&self.paths, &d);
    }

    pub fn log_diag(&self, e: DiagEntry) {
        if e.kind == DiagKind::Error {
            *self.last_error.lock() = e.error.clone().or(Some(e.summary.clone()));
        }
        self.diag.lock().push(e);
        self.notify();
    }

    /// Tells the settings window something changed.
    pub fn notify(&self) {
        let _ = self.handle.emit("wirl://state", ());
    }

    pub fn phase(&self) -> Phase {
        *self.phase.lock()
    }

    pub fn session_is(&self, id: u64) -> bool {
        self.session.load(Ordering::SeqCst) == id
    }

    pub fn model_path(&self, id: ModelId) -> std::path::PathBuf {
        self.paths.models.join(models::info(id).file)
    }

    pub fn model_ready(&self, id: ModelId) -> bool {
        self.model_path(id).exists()
    }

    /// Number of setup items needing attention (Home page / tray badge).
    pub fn attention_items(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        let s = self.settings();
        if s.cleanup_enabled && self.api_key.read().is_none() {
            v.push("apiKey");
        }
        if !crate::platform::microphone_allowed() || !crate::audio::has_input_device() {
            v.push("microphone");
        }
        if !self.model_ready(s.model) {
            v.push("model");
        }
        v
    }
}
