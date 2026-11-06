//! The settings window.

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const SETTINGS: &str = "settings";

/// Opens (or focuses) settings, optionally on a given page.
pub fn open_settings(app: &AppHandle, page: Option<&str>) {
    if let Some(w) = app.get_webview_window(SETTINGS) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        if let Some(p) = page {
            let _ = w.emit_to(SETTINGS, "wirl://navigate", p.to_string());
        }
        return;
    }
    let url = match page {
        Some(p) => format!("settings.html#{p}"),
        None => "settings.html".to_string(),
    };
    let built = WebviewWindowBuilder::new(app, SETTINGS, WebviewUrl::App(url.into()))
        .title("Wirl")
        .inner_size(980.0, 680.0)
        .min_inner_size(820.0, 560.0)
        .center()
        .background_color(tauri::window::Color(0x11, 0x11, 0x16, 0xff))
        .build();
    if let Err(e) = built {
        log::error!("open settings: {e}");
    }
}
