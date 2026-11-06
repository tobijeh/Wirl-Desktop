//! Files in the app data folder, plus secrets in the OS credential store
//! (Windows Credential Manager). The API key and phone secrets are never
//! written to disk by Wirl.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use wirl_core::dictionary::Dictionary;
use wirl_core::settings::Settings;

const SERVICE: &str = "Wirl";
const API_KEY_USER: &str = "anthropic-api-key";

#[derive(Debug, Clone)]
pub struct Paths {
    pub data: PathBuf,
    pub models: PathBuf,
    pub logs: PathBuf,
}

impl Paths {
    pub fn new(data: PathBuf) -> Self {
        let p = Paths {
            models: data.join("models"),
            logs: data.join("logs"),
            data,
        };
        for d in [&p.data, &p.models, &p.logs] {
            let _ = std::fs::create_dir_all(d);
        }
        p
    }
    pub fn settings(&self) -> PathBuf {
        self.data.join("settings.json")
    }
    pub fn dictionary(&self) -> PathBuf {
        self.data.join("dictionary.json")
    }
    pub fn device(&self) -> PathBuf {
        self.data.join("device.json")
    }
}

/// Writes atomically: temp file then rename, so a crash never leaves a
/// half-written settings or dictionary file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

pub fn load_settings(p: &Paths) -> Settings {
    std::fs::read_to_string(p.settings())
        .map(|s| Settings::from_json(&s))
        .unwrap_or_default()
}

pub fn save_settings(p: &Paths, s: &Settings) {
    if let Ok(json) = serde_json::to_vec_pretty(s) {
        if let Err(e) = write_atomic(&p.settings(), &json) {
            log::error!("saving settings: {e}");
        }
    }
}

pub fn load_dictionary(p: &Paths) -> Dictionary {
    std::fs::read_to_string(p.dictionary())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_dictionary(p: &Paths, d: &Dictionary) {
    if let Ok(json) = serde_json::to_vec_pretty(d) {
        if let Err(e) = write_atomic(&p.dictionary(), &json) {
            log::error!("saving dictionary: {e}");
        }
    }
}

/// A paired phone. The shared secret itself lives in the credential store.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedPhone {
    pub phone_id: String,
    pub name: String,
    pub paired_at: i64,
    pub last_sync: Option<i64>,
}

/// This computer's identity for sync, plus paired phone metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub device_id: String,
    #[serde(default)]
    pub phones: Vec<PairedPhone>,
}

pub fn load_device(p: &Paths) -> DeviceInfo {
    let d = std::fs::read_to_string(p.device())
        .ok()
        .and_then(|s| serde_json::from_str::<DeviceInfo>(&s).ok());
    match d {
        Some(d) => d,
        None => {
            let d = DeviceInfo {
                device_id: uuid::Uuid::new_v4().to_string(),
                phones: vec![],
            };
            save_device(p, &d);
            d
        }
    }
}

pub fn save_device(p: &Paths, d: &DeviceInfo) {
    if let Ok(json) = serde_json::to_vec_pretty(d) {
        let _ = write_atomic(&p.device(), &json);
    }
}

// ---------- secrets ----------

static SECRET_LOCK: Mutex<()> = Mutex::new(());

fn entry(user: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, user).map_err(|e| e.to_string())
}

pub fn get_api_key() -> Option<String> {
    let _g = SECRET_LOCK.lock();
    entry(API_KEY_USER)
        .ok()?
        .get_password()
        .ok()
        .filter(|k| !k.trim().is_empty())
}

pub fn set_api_key(key: &str) -> Result<(), String> {
    let _g = SECRET_LOCK.lock();
    let e = entry(API_KEY_USER)?;
    if key.trim().is_empty() {
        let _ = e.delete_credential();
        Ok(())
    } else {
        e.set_password(key.trim()).map_err(|e| e.to_string())
    }
}

/// "Saved, ending in XXXX".
pub fn api_key_hint(key: &str) -> String {
    let k = key.trim();
    let tail: String = k.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("Saved, ending in {tail}")
}

fn phone_user(phone_id: &str) -> String {
    format!("phone-{phone_id}")
}

pub fn get_phone_secret(phone_id: &str) -> Option<String> {
    let _g = SECRET_LOCK.lock();
    entry(&phone_user(phone_id)).ok()?.get_password().ok()
}

pub fn set_phone_secret(phone_id: &str, secret: &str) -> Result<(), String> {
    let _g = SECRET_LOCK.lock();
    entry(&phone_user(phone_id))?
        .set_password(secret)
        .map_err(|e| e.to_string())
}

pub fn delete_phone_secret(phone_id: &str) {
    let _g = SECRET_LOCK.lock();
    if let Ok(e) = entry(&phone_user(phone_id)) {
        let _ = e.delete_credential();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_hint_shows_last_four() {
        assert_eq!(api_key_hint("sk-ant-abc123WXYZ"), "Saved, ending in WXYZ");
    }

    #[test]
    fn device_info_created_once() {
        let dir = std::env::temp_dir().join(format!("wirl-dev-{}", std::process::id()));
        let p = Paths::new(dir.clone());
        let a = load_device(&p);
        let b = load_device(&p);
        assert_eq!(a.device_id, b.device_id);
        let _ = std::fs::remove_dir_all(dir);
    }
}
