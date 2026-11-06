//! Global hotkeys. Dictate and Clean up selection are registered while Wirl
//! is on; Cancel only while recording. Registration failures mean another
//! app owns the combination and are shown as amber warnings.

use crate::app;
use crate::dictation;
use parking_lot::Mutex;
use std::str::FromStr;
use std::sync::atomic::Ordering;
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};
use wirl_core::hotkey::{find_conflict, registration_conflict, Action, Hotkey};

static REGISTERED: Mutex<Vec<(u32, Action, Shortcut)>> = Mutex::new(Vec::new());

fn to_shortcut(h: &Hotkey) -> Option<Shortcut> {
    Shortcut::from_str(&h.to_accelerator()).ok()
}

pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|_app: &AppHandle, shortcut: &Shortcut, ev: ShortcutEvent| {
            let action = REGISTERED
                .lock()
                .iter()
                .find(|(id, _, _)| *id == shortcut.id())
                .map(|(_, a, _)| *a);
            let Some(action) = action else { return };
            match (action, ev.state) {
                (Action::Dictate, ShortcutState::Pressed) => dictation::on_dictate_pressed(),
                (Action::Dictate, ShortcutState::Released) => dictation::on_dictate_released(),
                (Action::Cancel, ShortcutState::Pressed) => dictation::cancel(),
                (Action::CleanSelection, ShortcutState::Pressed) => dictation::on_clean_selection(),
                _ => {}
            }
        })
        .build()
}

fn register_one(app: &AppHandle, action: Action, h: &Hotkey) -> Result<(), String> {
    let sc = to_shortcut(h).ok_or_else(|| format!("{h} can't be used as a shortcut"))?;
    app.global_shortcut().register(sc).map_err(|e| e.to_string())?;
    REGISTERED.lock().push((sc.id(), action, sc));
    Ok(())
}

fn unregister_action(app: &AppHandle, action: Action) {
    let mut reg = REGISTERED.lock();
    reg.retain(|(_, a, sc)| {
        if *a == action {
            let _ = app.global_shortcut().unregister(*sc);
            false
        } else {
            true
        }
    });
}

/// (Re)registers Dictate and Clean up selection from settings and
/// recomputes conflict warnings.
pub fn register_all() {
    let Some(a) = app::try_get() else { return };
    let app = a.handle.clone();
    let s = a.settings();
    let _ = app.global_shortcut().unregister_all();
    REGISTERED.lock().clear();
    let all = s.hotkeys.all();
    let mut conflicts = Vec::new();
    for (action, h) in &all {
        if let Some(c) = find_conflict(h, *action, &all) {
            conflicts.push((*action, c));
        }
    }
    if s.enabled && !a.capturing_hotkey.load(Ordering::Relaxed) {
        for action in [Action::Dictate, Action::CleanSelection] {
            let h = s.hotkeys.get(action);
            if let Err(e) = register_one(&app, action, h) {
                log::warn!("register {h}: {e}");
                if !conflicts.iter().any(|(a, _)| *a == action) {
                    conflicts.push((action, registration_conflict(h)));
                }
            }
        }
    }
    *a.hotkey_conflicts.lock() = conflicts;
    a.trigger.lock().hold_to_talk = s.hold_to_talk;
    a.notify();
}

/// Cancel is only registered while recording, so Esc works normally otherwise.
pub fn set_cancel_active(active: bool) {
    let Some(a) = app::try_get() else { return };
    let app = a.handle.clone();
    unregister_action(&app, Action::Cancel);
    if active {
        let h = a.settings().hotkeys.cancel;
        if let Err(e) = register_one(&app, Action::Cancel, &h) {
            log::warn!("cancel hotkey {h}: {e}");
        }
    }
}

/// While the Hotkeys page records a new shortcut, Wirl's own shortcuts are
/// paused so the keys reach the page.
pub fn set_capturing(capturing: bool) {
    let Some(a) = app::try_get() else { return };
    a.capturing_hotkey.store(capturing, Ordering::Relaxed);
    if capturing {
        let _ = a.handle.global_shortcut().unregister_all();
        REGISTERED.lock().clear();
    } else {
        register_all();
    }
}

/// Test whether Windows will let us register a combination (i.e. no other
/// app owns it). Wirl's own current registrations are skipped.
pub fn probe_registration(h: &Hotkey) -> bool {
    let Some(a) = app::try_get() else { return true };
    let Some(sc) = to_shortcut(h) else { return false };
    let gs = a.handle.global_shortcut();
    if gs.is_registered(sc) {
        return true;
    }
    match gs.register(sc) {
        Ok(()) => {
            let _ = gs.unregister(sc);
            true
        }
        Err(_) => false,
    }
}
