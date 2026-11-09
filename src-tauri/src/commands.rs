//! Commands called by the settings window. Everything autosaves.

use crate::app::{self, DownloadState, Phase};
use crate::{dictation, hotkeys, storage, sync_server, tray, updater};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;
use wirl_core::dictionary::split_sounds_like;
use wirl_core::hotkey::{default_for, find_conflict, Action, Hotkey, RecordResult, ShortcutRecorder};
use wirl_core::models::{self, human_size, MODELS};
use wirl_core::settings::{language_label, ModelId, Settings, LANGUAGES};

type CmdResult<T> = Result<T, String>;

fn action_key(a: Action) -> &'static str {
    match a {
        Action::Dictate => "dictate",
        Action::Cancel => "cancel",
        Action::CleanSelection => "cleanSelection",
    }
}

#[tauri::command]
pub fn get_state() -> Value {
    let a = app::get();
    let s = a.settings();
    let key = a.api_key.read().clone();
    let downloads = a.downloads.lock().clone();
    let models: Vec<Value> = MODELS
        .iter()
        .map(|m| {
            let d = downloads.get(&m.id).cloned().unwrap_or_default();
            json!({
                "id": m.id, "name": m.name, "description": m.description,
                "sizeLabel": human_size(m.size), "ready": a.model_ready(m.id),
                "download": d, "englishOnly": m.english_only,
            })
        })
        .collect();
    let dict = a.dict.lock();
    let entries: Vec<_> = dict.visible().into_iter().cloned().collect();
    let count = dict.count();
    drop(dict);
    let phones = a.device.lock().phones.clone();
    let conflicts: serde_json::Map<String, Value> = a
        .hotkey_conflicts
        .lock()
        .iter()
        .map(|(act, c)| (action_key(*act).to_string(), json!(c.message())))
        .collect();
    let attention = a.attention_items();
    let mic_ok = !attention.contains(&"microphone");
    let icons = a.icons.read().clone();
    let diagnostics = if s.developer_options {
        json!({ "entries": a.diag.lock().entries(), "sidecar": a.sidecar.status(), "dataFolder": a.paths.data })
    } else {
        Value::Null
    };
    json!({
        "version": a.handle.package_info().version.to_string(),
        "settings": s,
        "phase": a.phase(),
        "apiKey": { "saved": key.is_some(), "hint": key.as_deref().map(storage::api_key_hint) },
        "attention": attention,
        "setup": {
            "apiKey": key.is_some(),
            "microphone": mic_ok,
            "model": a.model_ready(s.model),
            "paired": !phones.is_empty(),
        },
        "models": models,
        "modelSummary": format!("{} · {}", models::info(s.model).name, language_label(&models::effective_language(s.model, &s.language))),
        "languages": LANGUAGES,
        "dictionary": { "count": count, "entries": entries },
        "phones": phones,
        "hotkeys": {
            "dictate": s.hotkeys.dictate.to_string(),
            "cancel": s.hotkeys.cancel.to_string(),
            "cleanSelection": s.hotkeys.clean_selection.to_string(),
            "conflicts": conflicts,
        },
        "icons": icons,
        "update": a.update.lock().clone(),
        "diagnostics": diagnostics,
        "lastError": a.last_error.lock().clone(),
        "computerName": sync_server::computer_name(),
        "pairingPending": a.pairing.lock().as_ref().is_some_and(|p| wirl_core::dictionary::now_ms() - p.created_ms < app::PAIRING_TTL_MS),
        "syncRejectedMs": *a.sync_rejected_ms.lock(),
    })
}

/// Turns Wirl on/off (off disables hotkeys).
pub fn apply_enabled(enabled: bool) {
    let a = app::get();
    a.settings.write().enabled = enabled;
    a.save_settings();
    if !enabled {
        dictation::cancel();
    }
    hotkeys::register_all();
    tray::refresh();
}

fn apply_autostart(app: &AppHandle, on: bool) {
    let al = app.autolaunch();
    let r = if on { al.enable() } else { al.disable() };
    if let Err(e) = r {
        log::warn!("autostart: {e}");
    }
}

/// Merges a partial settings object, saves, and applies side effects.
#[tauri::command]
pub fn update_settings(app: AppHandle, patch: Value) -> CmdResult<()> {
    let a = app::get();
    let old = a.settings();
    let mut v = serde_json::to_value(&old).map_err(|e| e.to_string())?;
    if let (Value::Object(base), Value::Object(p)) = (&mut v, patch) {
        for (k, val) in p {
            // Hotkeys are only changed through the recorder.
            if k != "hotkeys" {
                base.insert(k, val);
            }
        }
    }
    let new: Settings = serde_json::from_value(v).map_err(|e| e.to_string())?;
    *a.settings.write() = new.clone();
    a.save_settings();
    if new.enabled != old.enabled || new.hold_to_talk != old.hold_to_talk {
        hotkeys::register_all();
    }
    if new.enabled != old.enabled && !new.enabled {
        dictation::cancel();
    }
    if new.start_with_windows != old.start_with_windows {
        apply_autostart(&app, new.start_with_windows);
    }
    if new.model != old.model {
        ensure_model(new.model);
    }
    tray::refresh();
    Ok(())
}

/// Loads the model if downloaded, otherwise starts the download.
pub fn ensure_model(id: ModelId) {
    let a = app::get();
    if a.model_ready(id) {
        let path = a.model_path(id).to_string_lossy().to_string();
        let sc = a.sidecar.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = sc.load(&path).await {
                log::error!("load model: {e}");
            }
        });
    } else {
        let _ = download_model(id);
    }
}

#[tauri::command]
pub fn download_model(id: ModelId) -> CmdResult<()> {
    let a = app::get().clone();
    if a.downloads.lock().get(&id).map(|d| d.active).unwrap_or(false) {
        return Ok(());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    a.download_cancel.lock().insert(id, cancel.clone());
    a.downloads.lock().insert(
        id,
        DownloadState {
            active: true,
            ..Default::default()
        },
    );
    a.notify();
    tauri::async_runtime::spawn(async move {
        let info = models::info(id);
        let a2 = a.clone();
        let mut last_emit = std::time::Instant::now();
        let r = models::download::download(&a.http, info, &a.paths.models, cancel, move |done, total| {
            {
                let mut d = a2.downloads.lock();
                let e = d.entry(id).or_default();
                e.done = done;
                e.total = total;
            }
            if last_emit.elapsed().as_millis() > 200 || done == total {
                last_emit = std::time::Instant::now();
                let _ = a2
                    .handle
                    .emit("wirl://download", json!({ "id": id, "done": done, "total": total }));
            }
        })
        .await;
        {
            let mut d = a.downloads.lock();
            let e = d.entry(id).or_default();
            e.active = false;
            e.error = r.as_ref().err().map(|e| e.to_string());
        }
        match r {
            Ok(_) => {
                if a.settings().model == id {
                    ensure_model(id);
                }
            }
            Err(e) => log::warn!("download {}: {e}", info.file),
        }
        tray::refresh();
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_download(id: ModelId) {
    if let Some(c) = app::get().download_cancel.lock().get(&id) {
        c.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
pub async fn set_api_key(key: String) -> CmdResult<String> {
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err("Paste your API key first.".into());
    }
    // Check the key with a free request before saving.
    let a = app::get();
    let resp = a
        .http
        .get("https://api.anthropic.com/v1/models?limit=1")
        .header("x-api-key", &key)
        .header("anthropic-version", wirl_core::prompt::ANTHROPIC_VERSION)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;
    let mut note = String::new();
    match resp {
        Ok(r) if r.status().as_u16() == 401 || r.status().as_u16() == 403 => {
            return Err("That key wasn't accepted. Check it and try again.".into());
        }
        Ok(_) => {}
        Err(_) => note = " (couldn't check it: no internet)".into(),
    }
    storage::set_api_key(&key).map_err(|e| format!("Couldn't save the key: {e}"))?;
    *a.api_key.write() = Some(key.clone());
    tray::refresh();
    Ok(format!("{}{note}", storage::api_key_hint(&key)))
}

#[tauri::command]
pub fn clear_api_key() -> CmdResult<()> {
    storage::set_api_key("")?;
    *app::get().api_key.write() = None;
    tray::refresh();
    Ok(())
}

// ---------- dictionary ----------

#[tauri::command]
pub fn dict_add(term: String, sounds_like: String, note: String) -> CmdResult<()> {
    let a = app::get();
    a.dict.lock().add(&term, split_sounds_like(&sounds_like), &note)?;
    a.save_dictionary();
    a.notify();
    Ok(())
}

#[tauri::command]
pub fn dict_edit(id: String, term: String, sounds_like: String, note: String) -> CmdResult<()> {
    let a = app::get();
    a.dict.lock().edit(&id, &term, split_sounds_like(&sounds_like), &note)?;
    a.save_dictionary();
    a.notify();
    Ok(())
}

#[tauri::command]
pub fn dict_delete(id: String) -> CmdResult<()> {
    let a = app::get();
    a.dict.lock().delete(&id);
    a.save_dictionary();
    a.notify();
    Ok(())
}

#[tauri::command]
pub async fn dict_import(app: AppHandle) -> CmdResult<Option<String>> {
    let Some(path) = app
        .dialog()
        .file()
        .add_filter("Dictionary", &["json"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("Couldn't read the file: {e}"))?;
    let a = app::get();
    let s = a.dict.lock().import_json(&text)?;
    a.save_dictionary();
    a.notify();
    Ok(Some(format!("Added {}, updated {}", s.added, s.updated)))
}

#[tauri::command]
pub async fn dict_export(app: AppHandle) -> CmdResult<Option<String>> {
    let Some(path) = app
        .dialog()
        .file()
        .add_filter("Dictionary", &["json"])
        .set_file_name("wirl-dictionary.json")
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    let json = app::get().dict.lock().export_json();
    std::fs::write(&path, json).map_err(|e| format!("Couldn't save: {e}"))?;
    Ok(Some("Exported".into()))
}

// ---------- pairing ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingInfo {
    svg: String,
    expires_ms: i64,
}

#[tauri::command]
pub fn pair_begin() -> CmdResult<PairingInfo> {
    let (_json, svg) = sync_server::begin_pairing()?;
    Ok(PairingInfo {
        svg,
        expires_ms: wirl_core::dictionary::now_ms() + app::PAIRING_TTL_MS,
    })
}

/// Withdraws the pairing code shown on screen (an explicit "Cancel pairing").
/// Closing the QR panel doesn't call this: the code stays valid until a phone
/// uses it or it expires, because the phone's first sync comes a little later.
#[tauri::command]
pub fn pair_cancel() {
    let a = app::get();
    *a.pairing.lock() = None;
    a.notify();
}

#[tauri::command]
pub fn unpair(phone_id: String) {
    sync_server::unpair(&phone_id);
}

// ---------- hotkeys ----------

#[tauri::command]
pub fn hotkey_capture(active: bool) {
    hotkeys::set_capturing(active);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyResult {
    status: &'static str,
    display: Option<String>,
    warning: Option<String>,
    error: Option<String>,
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn hotkey_key(action: Action, ctrl: bool, alt: bool, shift: bool, win: bool, code: String) -> KeyResult {
    let a = app::get();
    let others = a.settings().hotkeys.all();
    let mut rec = ShortcutRecorder::default();
    rec.start(action);
    match rec.key_down(ctrl, alt, shift, win, &code, &others) {
        RecordResult::Waiting => KeyResult {
            status: "waiting",
            display: None,
            warning: None,
            error: None,
        },
        RecordResult::Rejected(e) => KeyResult {
            status: "rejected",
            display: None,
            warning: None,
            error: Some(e),
        },
        RecordResult::Saved {
            action,
            hotkey,
            mut warning,
        } => {
            if warning.is_none() && action != Action::Cancel && !hotkeys::probe_registration(&hotkey) {
                warning = Some(wirl_core::hotkey::registration_conflict(&hotkey));
            }
            a.settings.write().hotkeys.set(action, hotkey.clone());
            a.save_settings();
            KeyResult {
                status: "saved",
                display: Some(hotkey.to_string()),
                warning: warning.map(|w| w.message()),
                error: None,
            }
        }
    }
}

#[tauri::command]
pub fn hotkeys_reset() {
    let a = app::get();
    {
        let mut s = a.settings.write();
        for act in [Action::Dictate, Action::Cancel, Action::CleanSelection] {
            s.hotkeys.set(act, default_for(act));
        }
    }
    a.save_settings();
    hotkeys::register_all();
}

/// Conflict check for display only (no saving).
#[tauri::command]
pub fn hotkey_check(action: Action, combo: String) -> Option<String> {
    let h = Hotkey::parse(&combo).ok()?;
    let others = app::get().settings().hotkeys.all();
    find_conflict(&h, action, &others).map(|c| c.message())
}

// ---------- misc ----------

#[tauri::command]
pub fn reset_defaults(app: AppHandle) {
    let a = app::get();
    let new = a.settings().reset_to_defaults();
    *a.settings.write() = new.clone();
    a.save_settings();
    apply_autostart(&app, new.start_with_windows);
    hotkeys::register_all();
    ensure_model(new.model);
    tray::refresh();
}

#[tauri::command]
pub fn check_updates() {
    updater::check_now(true);
}

#[tauri::command]
pub fn restart_to_update() {
    updater::restart_to_update();
}

/// Setup's "Test dictation": toggles a dictation into the focused text box.
#[tauri::command]
pub fn test_dictation() {
    let a = app::get();
    match a.phase() {
        Phase::Idle => {
            a.trigger.lock().reset();
            dictation::on_dictate_pressed();
            dictation::on_dictate_released(); // a tap: toggle mode
        }
        Phase::Recording => dictation::stop(),
        _ => {}
    }
}

#[tauri::command]
pub fn reveal_developer_options() {
    let a = app::get();
    a.settings.write().developer_options = true;
    a.save_settings();
    a.notify();
}

#[tauri::command]
pub fn open_link(url: String) -> CmdResult<()> {
    if !(url.starts_with("https://") || url.starts_with("ms-settings:")) {
        return Err("unsupported link".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", &url])
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(&url)
        .spawn()
        .map_err(|e| e.to_string())?;
    #[cfg(all(unix, not(target_os = "macos")))]
    std::process::Command::new("xdg-open")
        .arg(&url)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn mark_first_run_done() {
    let a = app::get();
    a.settings.write().first_run_done = true;
    a.save_settings();
    a.notify();
}
