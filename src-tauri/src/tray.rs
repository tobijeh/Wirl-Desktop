//! System tray icon and menu.

use crate::app::{self, Phase};
use crate::{dictation, updater, windows};
use image::RgbaImage;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Wry};
use wirl_core::icons::{tray_image, TrayState, PROCESSING_FRAMES};

const TRAY_ID: &str = "main";
const TRAY_PX: u32 = 32;

struct Items {
    on: CheckMenuItem<Wry>,
    update: MenuItem<Wry>,
}

static ITEMS: Mutex<Option<Items>> = Mutex::new(None);
static FRAME: AtomicUsize = AtomicUsize::new(0);
static BASE: Mutex<Option<(String, RgbaImage)>> = Mutex::new(None);

/// The selected icon as provided, decoded from the bundled assets.
fn base_icon(app: &AppHandle) -> Option<RgbaImage> {
    let file = crate::pill::icon_file()?;
    if let Some((f, img)) = BASE.lock().as_ref() {
        if *f == file {
            return Some(img.clone());
        }
    }
    let asset = app.asset_resolver().get(format!("assets/icons/{file}"))?;
    let img = image::load_from_memory(&asset.bytes).ok()?.to_rgba8();
    *BASE.lock() = Some((file, img.clone()));
    Some(img)
}

pub fn state() -> TrayState {
    let Some(a) = app::try_get() else {
        return TrayState::Ready;
    };
    match a.phase() {
        Phase::Recording => return TrayState::Recording,
        Phase::Processing | Phase::CleaningSelection => return TrayState::Processing,
        Phase::Idle => {}
    }
    if !a.settings().enabled {
        TrayState::Off
    } else if !a.attention_items().is_empty() {
        TrayState::NeedsAttention
    } else if a.update.lock().ready_version.is_some() {
        TrayState::UpdateReady
    } else {
        TrayState::Ready
    }
}

fn tooltip(st: TrayState) -> &'static str {
    match st {
        TrayState::Ready => "Wirl is on",
        TrayState::Recording => "Wirl is listening",
        TrayState::Processing => "Wirl is working",
        TrayState::Off => "Wirl is off",
        TrayState::NeedsAttention => "Wirl needs attention",
        TrayState::UpdateReady => "Wirl update ready",
    }
}

fn image_for(app: &AppHandle, st: TrayState, frame: usize) -> Option<Image<'static>> {
    let base = base_icon(app)?;
    let img = tray_image(&base, TRAY_PX, st, frame);
    let (w, h) = img.dimensions();
    Some(Image::new_owned(img.into_raw(), w, h))
}

fn tray(app: &AppHandle) -> Option<TrayIcon> {
    app.tray_by_id(TRAY_ID)
}

/// Re-renders icon, tooltip and menu labels from current state.
pub fn refresh() {
    let Some(a) = app::try_get() else { return };
    let app = a.handle.clone();
    let st = state();
    if let Some(t) = tray(&app) {
        if let Some(img) = image_for(&app, st, FRAME.load(Ordering::Relaxed)) {
            let _ = t.set_icon(Some(img));
        }
        let _ = t.set_tooltip(Some(tooltip(st)));
    }
    if let Some(items) = ITEMS.lock().as_ref() {
        let on = a.settings().enabled;
        let _ = items.on.set_checked(on);
        let _ = items.on.set_text(if on { "Wirl is on" } else { "Wirl is off" });
        let ready = a.update.lock().ready_version.is_some();
        let _ = items.update.set_text(if ready {
            "Restart to update"
        } else {
            "Check for updates"
        });
    }
    a.notify();
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let on = CheckMenuItem::with_id(app, "toggle", "Wirl is on", true, true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let update = MenuItem::with_id(app, "update", "Check for updates", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&on, &settings, &update, &sep, &quit])?;
    *ITEMS.lock() = Some(Items { on, update });

    let mut b = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Wirl")
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            "toggle" => {
                let a = app::get();
                let enabled = !a.settings().enabled;
                crate::commands::apply_enabled(enabled);
            }
            "settings" => windows::open_settings(app, None),
            "update" => {
                if app::get().update.lock().ready_version.is_some() {
                    updater::restart_to_update();
                } else {
                    windows::open_settings(app, Some("about"));
                    updater::check_now(true);
                }
            }
            "quit" => {
                dictation::cancel();
                updater::quit_installing_if_ready(app);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                windows::open_settings(tray.app_handle(), None);
            }
        });
    if let Some(img) = image_for(app, TrayState::Ready, 0) {
        b = b.icon(img);
    } else if let Some(icon) = app.default_window_icon() {
        b = b.icon(icon.clone());
    }
    b.build(app)?;
    // Processing animation.
    tauri::async_runtime::spawn(async {
        let mut tick = tokio::time::interval(Duration::from_millis(140));
        loop {
            tick.tick().await;
            if state() == TrayState::Processing {
                let f = (FRAME.fetch_add(1, Ordering::Relaxed) + 1) % PROCESSING_FRAMES;
                if let Some(a) = app::try_get() {
                    if let (Some(t), Some(img)) = (tray(&a.handle), image_for(&a.handle, TrayState::Processing, f)) {
                        let _ = t.set_icon(Some(img));
                    }
                }
            }
        }
    });
    Ok(())
}
