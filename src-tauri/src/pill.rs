//! The pill window: shown only while Wirl is doing something, never
//! focused, always on top, on the monitor with the active window.

use crate::app;
use crate::platform;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use wirl_core::pill::{effective_state, place, PillPosition, PillState};

pub const LABEL: &str = "pill";

static GENERATION: AtomicU64 = AtomicU64::new(0);
static CURRENT: Mutex<Option<PillState>> = Mutex::new(None);

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PillPayload {
    state: PillState,
    icon: Option<String>,
    size: wirl_core::pill::PillSize,
}

pub fn create(app: &tauri::AppHandle) -> tauri::Result<WebviewWindow> {
    let w = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("pill.html".into()))
        .title("Wirl")
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .visible(false)
        .inner_size(240.0, 48.0)
        .build()?;
    platform::prepare_pill_window(&w);
    Ok(w)
}

fn window() -> Option<WebviewWindow> {
    app::try_get()?.handle.get_webview_window(LABEL)
}

pub fn current() -> PillState {
    CURRENT.lock().clone().unwrap_or(PillState::Hidden)
}

/// Selected icon's file name inside ui/assets/icons.
pub fn icon_file() -> Option<String> {
    let a = app::try_get()?;
    let set = a.icons.read().clone()?;
    let chosen = a.settings().icon;
    let id = if chosen.is_empty() { set.default.clone() } else { chosen };
    set.icons
        .iter()
        .find(|i| i.id == id)
        .or_else(|| set.icons.first())
        .map(|i| i.file.clone())
}

pub fn set(state: PillState) {
    let Some(a) = app::try_get() else { return };
    let s = a.settings();
    let state = effective_state(state, s.status_messages);
    let gen = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    *CURRENT.lock() = Some(state.clone());
    let Some(w) = window() else { return };
    if !state.visible() {
        platform::hide_pill_window(&w);
        return;
    }
    let payload = PillPayload {
        state: state.clone(),
        icon: icon_file(),
        size: s.pill_size,
    };
    let _ = w.emit_to(LABEL, "wirl://pill", payload);
    position_and_show(&w, s.pill_position, s.pill_size);
    if let Some(ms) = state.auto_hide_ms() {
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            if GENERATION.load(Ordering::SeqCst) == gen {
                // Let the page fade out, then hide the window.
                if let Some(w) = window() {
                    let _ = w.emit_to(LABEL, "wirl://pill-fade", ());
                    tokio::time::sleep(Duration::from_millis(220)).await;
                    if GENERATION.load(Ordering::SeqCst) == gen {
                        *CURRENT.lock() = Some(PillState::Hidden);
                        platform::hide_pill_window(&w);
                    }
                }
            }
        });
    }
}

fn position_and_show(w: &WebviewWindow, position: PillPosition, size: wirl_core::pill::PillSize) {
    let (work, cursor) = platform::placement_context();
    let (lw, lh) = size.dimensions();
    // Scale for the monitor the pill will be on.
    let monitors = w.available_monitors().unwrap_or_default();
    let work = work.or_else(|| {
        w.primary_monitor().ok().flatten().map(|m| wirl_core::pill::Rect {
            x: m.position().x as f64,
            y: m.position().y as f64,
            w: m.size().width as f64,
            h: m.size().height as f64,
        })
    });
    let Some(work) = work else { return };
    let cx = work.x + work.w / 2.0;
    let cy = work.y + work.h / 2.0;
    let scale = monitors
        .iter()
        .find(|m| {
            let p = m.position();
            let s = m.size();
            cx >= p.x as f64
                && cx < (p.x + s.width as i32) as f64
                && cy >= p.y as f64
                && cy < (p.y + s.height as i32) as f64
        })
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);
    let (pw, ph) = (lw * scale, lh * scale);
    let (x, y) = place(position, work, cursor, pw, ph);
    platform::show_pill_window(w, x as i32, y as i32, pw.round() as u32, ph.round() as u32);
}

/// Quick switch while recording: shows the model's tag (EN, EN+, 粵, 普) in
/// place of the icon, and its name, for a moment; then the usual recording
/// pill again, unless something else changed the pill meanwhile.
pub fn flash_switched_recording(tag: &str, name: &str, cancel_hotkey: &str) {
    set(PillState::recording_switched(tag, name));
    let gen = GENERATION.load(Ordering::SeqCst);
    let back = PillState::recording(cancel_hotkey);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(wirl_core::pill::TAG_FLASH_MS)).await;
        if GENERATION.load(Ordering::SeqCst) == gen {
            set(back);
        }
    });
}

/// Live input level for the wave while recording.
pub fn level(rms: f32) {
    if let Some(w) = window() {
        let _ = w.emit_to(LABEL, "wirl://level", rms);
    }
}
