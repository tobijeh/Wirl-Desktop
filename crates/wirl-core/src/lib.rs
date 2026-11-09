//! Platform-independent logic for Wirl. Everything here is unit tested and
//! compiles on any OS; Windows-specific code lives in `src-tauri`.

pub mod audio;
#[cfg(feature = "net")]
pub mod claude;
pub mod diagnostics;
pub mod dictionary;
pub mod filter;
pub mod hotkey;
pub mod icons;
pub mod insert;
pub mod ipc;
pub mod models;
pub mod onboarding;
pub mod pill;
#[cfg(feature = "net")]
pub mod pipeline;
pub mod prompt;
pub mod settings;
pub mod sync;
pub mod watchdog;
