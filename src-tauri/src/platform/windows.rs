//! Windows implementation: clipboard (all formats), SendInput paste/copy,
//! elevation check, monitor/cursor, no-activate pill window, media pause.

use std::ffi::c_void;
use std::time::{Duration, Instant};
use windows::core::{HSTRING, PCWSTR};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager as MediaManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HGLOBAL, HWND, POINT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RIGHT, VK_RMENU,
    VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetWindowLongPtrW, GetWindowThreadProcessId, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, GWL_EXSTYLE, HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT,
};
use wirl_core::insert::PasteError;
use wirl_core::pill::Rect;

const CF_UNICODETEXT: u32 = 13;
/// GDI/handle formats that can't be copied as plain memory.
const SKIP_FORMATS: &[u32] = &[2, 3, 9, 14, 0x80, 0x82, 0x83, 0x8E];
const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ClipboardSnapshot(Vec<(u32, Vec<u8>)>);

struct ClipGuard;
impl Drop for ClipGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

fn open_clipboard() -> Option<ClipGuard> {
    for _ in 0..20 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            return Some(ClipGuard);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

unsafe fn read_hglobal(h: HANDLE) -> Option<Vec<u8>> {
    let g = HGLOBAL(h.0);
    let size = GlobalSize(g);
    if size == 0 || size > MAX_SNAPSHOT_BYTES {
        return None;
    }
    let p = GlobalLock(g) as *const u8;
    if p.is_null() {
        return None;
    }
    let v = std::slice::from_raw_parts(p, size).to_vec();
    let _ = GlobalUnlock(g);
    Some(v)
}

unsafe fn set_bytes(format: u32, bytes: &[u8]) -> Result<(), String> {
    let g = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)).map_err(|e| e.to_string())?;
    let p = GlobalLock(g) as *mut u8;
    if p.is_null() {
        return Err("GlobalLock failed".into());
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    let _ = GlobalUnlock(g);
    SetClipboardData(format, Some(HANDLE(g.0))).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn clipboard_snapshot() -> Option<ClipboardSnapshot> {
    let _g = open_clipboard()?;
    let mut out = Vec::new();
    let mut total = 0usize;
    let mut fmt = 0u32;
    unsafe {
        loop {
            fmt = EnumClipboardFormats(fmt);
            if fmt == 0 {
                break;
            }
            if SKIP_FORMATS.contains(&fmt) {
                continue;
            }
            if let Ok(h) = GetClipboardData(fmt) {
                if let Some(bytes) = read_hglobal(h) {
                    total += bytes.len();
                    if total > MAX_SNAPSHOT_BYTES {
                        break;
                    }
                    out.push((fmt, bytes));
                }
            }
        }
    }
    Some(ClipboardSnapshot(out))
}

pub fn clipboard_restore(s: ClipboardSnapshot) -> Result<(), String> {
    let _g = open_clipboard().ok_or("clipboard busy")?;
    unsafe {
        EmptyClipboard().map_err(|e| e.to_string())?;
        for (fmt, bytes) in &s.0 {
            let _ = set_bytes(*fmt, bytes);
        }
    }
    Ok(())
}

fn registered(name: &str) -> u32 {
    let w = HSTRING::from(name);
    unsafe { RegisterClipboardFormatW(PCWSTR(w.as_ptr())) }
}

pub fn clipboard_set_text(text: &str) -> Result<(), String> {
    let _g = open_clipboard().ok_or("clipboard busy")?;
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    let bytes: Vec<u8> = wide.iter().flat_map(|c| c.to_le_bytes()).collect();
    unsafe {
        EmptyClipboard().map_err(|e| e.to_string())?;
        set_bytes(CF_UNICODETEXT, &bytes)?;
        // Keep dictations out of clipboard history and cloud clipboard.
        let zero = 0u32.to_le_bytes();
        let _ = set_bytes(registered("ExcludeClipboardContentFromMonitorProcessing"), &zero);
        let _ = set_bytes(registered("CanIncludeInClipboardHistory"), &zero);
        let _ = set_bytes(registered("CanUploadToCloudClipboard"), &zero);
    }
    Ok(())
}

pub fn clipboard_get_text() -> Option<String> {
    let _g = open_clipboard()?;
    unsafe {
        let h = GetClipboardData(CF_UNICODETEXT).ok()?;
        let bytes = read_hglobal(h)?;
        let wide: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|c| *c != 0)
            .collect();
        Some(String::from_utf16_lossy(&wide))
    }
}

pub fn clipboard_sequence() -> u64 {
    unsafe { GetClipboardSequenceNumber() as u64 }
}

fn token_elevated(process: HANDLE) -> Option<bool> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
        let mut elev = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let r = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elev as *mut _ as *mut c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        let _ = CloseHandle(token);
        r.ok()?;
        Some(elev.TokenIsElevated != 0)
    }
}

/// Windows blocks injected keys into elevated windows from a normal
/// process (UIPI) without reporting an error, so check first.
pub fn can_paste() -> Result<(), PasteError> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return Err(PasteError::NoTarget);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(fg, Some(&mut pid));
        if pid == 0 {
            return Err(PasteError::NoTarget);
        }
        let me_elevated = token_elevated(GetCurrentProcess()).unwrap_or(false);
        if me_elevated {
            return Ok(());
        }
        match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(h) => {
                let target = token_elevated(h);
                let _ = CloseHandle(h);
                match target {
                    Some(true) | None => Err(PasteError::Blocked),
                    Some(false) => Ok(()),
                }
            }
            // System or protected process: we can't inject into it.
            Err(_) => Err(PasteError::Blocked),
        }
    }
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    if vk == VK_LEFT || vk == VK_RIGHT {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send(inputs: &[INPUT]) -> Result<(), PasteError> {
    let n = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if n as usize == inputs.len() {
        Ok(())
    } else {
        Err(PasteError::Blocked)
    }
}

/// Sends Ctrl+<letter>.
pub fn send_ctrl_key(letter: char) -> Result<(), PasteError> {
    let vk = VIRTUAL_KEY(letter.to_ascii_uppercase() as u16);
    send(&[
        key(VK_CONTROL, false),
        key(vk, false),
        key(vk, true),
        key(VK_CONTROL, true),
    ])
}

fn is_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0 }
}

/// Waits until the user has let go of Ctrl/Alt/Shift/Win so our Ctrl+V
/// isn't combined with the hotkey's modifiers.
pub fn wait_modifiers_released(timeout: Duration) {
    let start = Instant::now();
    let mods = [
        VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN, VK_LMENU, VK_RMENU, VK_LSHIFT,
    ];
    while start.elapsed() < timeout {
        if !mods.iter().any(|m| is_down(*m)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Selects `chars` characters to the left of the caret (Shift+Left).
pub fn select_back(chars: usize) -> Result<(), PasteError> {
    let n = chars.min(5_000);
    let mut v = Vec::with_capacity(n * 2 + 2);
    v.push(key(VK_SHIFT, false));
    for _ in 0..n {
        v.push(key(VK_LEFT, false));
        v.push(key(VK_LEFT, true));
    }
    v.push(key(VK_SHIFT, true));
    send(&v)
}

/// Collapses a selection to its right end.
pub fn collapse_selection_right() {
    let _ = send(&[key(VK_RIGHT, false), key(VK_RIGHT, true)]);
}

pub fn active_work_area() -> Option<Rect> {
    unsafe {
        let fg = GetForegroundWindow();
        let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return None;
        }
        let r = mi.rcWork;
        Some(Rect {
            x: r.left as f64,
            y: r.top as f64,
            w: (r.right - r.left) as f64,
            h: (r.bottom - r.top) as f64,
        })
    }
}

pub fn cursor_pos() -> (f64, f64) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x as f64, p.y as f64)
}

fn hwnd_of(w: &tauri::WebviewWindow) -> Option<HWND> {
    w.hwnd().ok().map(|h| HWND(h.0))
}

/// The pill never takes focus: no-activate, tool window (no taskbar),
/// click-through and topmost.
pub fn prepare_pill_window(w: &tauri::WebviewWindow) {
    if let Some(h) = hwnd_of(w) {
        unsafe {
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
            let add = (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0 | WS_EX_TRANSPARENT.0) as isize;
            SetWindowLongPtrW(h, GWL_EXSTYLE, ex | add);
        }
    }
}

pub fn show_pill_window(w: &tauri::WebviewWindow, x: i32, y: i32, width: u32, height: u32) {
    if let Some(h) = hwnd_of(w) {
        unsafe {
            let _ = SetWindowPos(
                h,
                Some(HWND_TOPMOST),
                x,
                y,
                width as i32,
                height as i32,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }
}

pub fn hide_pill_window(w: &tauri::WebviewWindow) {
    if let Some(h) = hwnd_of(w) {
        unsafe {
            let _ = ShowWindow(h, SW_HIDE);
        }
    }
}

// ---------- media ----------

/// Apps we paused, by AppUserModelId, so only those are resumed.
#[derive(Debug, Default)]
pub struct MediaPauseToken(pub Vec<String>);

/// Pauses every session that is currently playing (Windows media controls).
pub fn pause_playing_media() -> MediaPauseToken {
    let mut paused = Vec::new();
    let mut run = || -> windows::core::Result<()> {
        let mgr = MediaManager::RequestAsync()?.get()?;
        let sessions = mgr.GetSessions()?;
        for i in 0..sessions.Size()? {
            let s = sessions.GetAt(i)?;
            if s.GetPlaybackInfo()?.PlaybackStatus()? == PlaybackStatus::Playing && s.TryPauseAsync()?.get()? {
                paused.push(s.SourceAppUserModelId()?.to_string());
            }
        }
        Ok(())
    };
    if let Err(e) = run() {
        log::warn!("media pause: {e}");
    }
    MediaPauseToken(paused)
}

pub fn resume_media(t: MediaPauseToken) {
    if t.0.is_empty() {
        return;
    }
    let run = || -> windows::core::Result<()> {
        let mgr = MediaManager::RequestAsync()?.get()?;
        let sessions = mgr.GetSessions()?;
        for i in 0..sessions.Size()? {
            let s = sessions.GetAt(i)?;
            let id = s.SourceAppUserModelId()?.to_string();
            if t.0.contains(&id) && s.GetPlaybackInfo()?.PlaybackStatus()? == PlaybackStatus::Paused {
                let _ = s.TryPlayAsync()?.get();
            }
        }
        Ok(())
    };
    if let Err(e) = run() {
        log::warn!("media resume: {e}");
    }
}

fn reg_string(path: &str, name: &str) -> Option<String> {
    let p = HSTRING::from(path);
    let n = HSTRING::from(name);
    let mut buf = [0u16; 64];
    let mut size = (buf.len() * 2) as u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(p.as_ptr()),
            PCWSTR(n.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut c_void),
            Some(&mut size),
        )
        .ok()
        .ok()?;
    }
    let len = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..len.min(buf.len())]))
}

/// False if Windows privacy settings block microphone access for desktop apps.
pub fn microphone_allowed() -> bool {
    let base = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";
    let global = reg_string(base, "Value");
    let desktop = reg_string(&format!(r"{base}\NonPackaged"), "Value");
    !(global.as_deref() == Some("Deny") || desktop.as_deref() == Some("Deny"))
}
