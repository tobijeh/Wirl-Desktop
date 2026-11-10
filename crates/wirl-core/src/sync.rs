//! Dictionary sync through an encrypted mailbox (protocol v2). See
//! docs/sync-protocol.md; the Android app implements the same protocol.
//!
//! Every device in a sync group shares one 32-byte secret (from the QR
//! code). From it each device derives the mailbox id, the mailbox token and
//! the encryption key. The relay only ever stores ciphertext.

use crate::dictionary::{merge_entries, Entry, TOMBSTONE_MAX_AGE_MS};
use aes_gcm::aead::{Aead, KeyInit, Payload as AeadPayload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD as B64URL};
use base64::Engine;
use hkdf::Hkdf;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub const PROTOCOL_VERSION: u32 = 2;
/// The mailbox relay used when a build doesn't override it.
pub const DEFAULT_RELAY: &str = "https://wirl-relay.wirl.workers.dev";
pub const INFO_MAILBOX: &[u8] = b"wirl-mailbox-v2";
pub const INFO_TOKEN: &[u8] = b"wirl-token-v2";
pub const INFO_KEY: &[u8] = b"wirl-sync-v2";
/// Devices not seen for this long are dropped from the device list.
pub const DEVICE_MAX_AGE_MS: i64 = 180 * 24 * 60 * 60 * 1000;
/// A device re-uploads after this long even when nothing changed, so the
/// others can show "Synced X ago" for it.
pub const HEARTBEAT_MS: i64 = 15 * 60 * 1000;

/// Contents of the pairing QR code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingQr {
    pub v: u32,
    /// The computer's name, for display.
    pub name: String,
    /// Base URL of the mailbox relay.
    pub relay: String,
    /// Standard base64 of the 32-byte group secret.
    pub secret: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SyncError {
    #[error("the sync secret is not valid")]
    BadSecret,
    /// The data doesn't decrypt: wrong secret or tampering.
    #[error("couldn't decrypt the synced dictionary")]
    Decrypt,
    #[error("unexpected data: {0}")]
    BadData(String),
}

/// Values derived from the group secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupKeys {
    /// 64 lowercase hex characters.
    pub mailbox: String,
    /// URL-safe base64 without padding (43 characters).
    pub token: String,
    pub key: [u8; 32],
}

fn hkdf32(ikm: &[u8], info: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, ikm);
    let mut okm = [0u8; 32];
    hk.expand(info, &mut okm).expect("32 bytes is a valid HKDF length");
    okm
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 32 random bytes, standard base64.
pub fn new_secret() -> String {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    B64.encode(b)
}

/// HKDF-SHA256 (no salt) of the 32 secret bytes with three fixed labels.
pub fn derive(secret_b64: &str) -> Result<GroupKeys, SyncError> {
    let ikm = B64.decode(secret_b64.trim()).map_err(|_| SyncError::BadSecret)?;
    if ikm.len() != 32 {
        return Err(SyncError::BadSecret);
    }
    Ok(GroupKeys {
        mailbox: hex(&hkdf32(&ikm, INFO_MAILBOX)),
        token: B64URL.encode(hkdf32(&ikm, INFO_TOKEN)),
        key: hkdf32(&ikm, INFO_KEY),
    })
}

/// SHA-256 of the token, hex. The relay stores this, never the token.
pub fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// One encrypted mailbox value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sealed {
    pub nonce: String,
    pub ciphertext: String,
}

/// AES-256-GCM, AAD = the mailbox id (ASCII). Fresh random 12-byte nonce.
pub fn encrypt(keys: &GroupKeys, plaintext: &[u8]) -> Sealed {
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    encrypt_with_nonce(keys, &nonce, plaintext)
}

/// Deterministic form, for test vectors only. Never reuse a nonce.
pub fn encrypt_with_nonce(keys: &GroupKeys, nonce: &[u8; 12], plaintext: &[u8]) -> Sealed {
    let cipher = Aes256Gcm::new((&keys.key).into());
    let ct = cipher
        .encrypt(
            Nonce::from_slice(nonce),
            AeadPayload {
                msg: plaintext,
                aad: keys.mailbox.as_bytes(),
            },
        )
        .expect("aes-gcm encryption cannot fail for in-memory buffers");
    Sealed {
        nonce: B64.encode(nonce),
        ciphertext: B64.encode(ct),
    }
}

pub fn decrypt(keys: &GroupKeys, sealed: &Sealed) -> Result<Vec<u8>, SyncError> {
    let nonce = B64.decode(&sealed.nonce).map_err(|_| SyncError::Decrypt)?;
    if nonce.len() != 12 {
        return Err(SyncError::Decrypt);
    }
    let ct = B64.decode(&sealed.ciphertext).map_err(|_| SyncError::Decrypt)?;
    Aes256Gcm::new((&keys.key).into())
        .decrypt(
            Nonce::from_slice(&nonce),
            AeadPayload {
                msg: &ct,
                aad: keys.mailbox.as_bytes(),
            },
        )
        .map_err(|_| SyncError::Decrypt)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceKind {
    Computer,
    Phone,
}

/// A device in the sync group, as shown on the others ("Synced 2 min ago").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    /// Milliseconds since the epoch of this device's last successful upload.
    pub last_sync: i64,
}

/// The decrypted mailbox content.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payload {
    #[serde(default)]
    pub entries: Vec<Entry>,
    #[serde(default)]
    pub devices: Vec<Device>,
}

pub fn seal_payload(keys: &GroupKeys, p: &Payload) -> Sealed {
    encrypt(keys, &serde_json::to_vec(p).expect("payload serializes"))
}

pub fn open_payload(keys: &GroupKeys, sealed: &Sealed) -> Result<Payload, SyncError> {
    let plain = decrypt(keys, sealed)?;
    serde_json::from_slice(&plain).map_err(|e| SyncError::BadData(e.to_string()))
}

/// Per id: the newer `lastSync` wins. Old devices are dropped. Sorted by id.
pub fn merge_devices(a: &[Device], b: &[Device], now: i64) -> Vec<Device> {
    let mut map: HashMap<&str, &Device> = HashMap::new();
    for d in a.iter().chain(b.iter()) {
        match map.get(d.id.as_str()) {
            Some(cur) if cur.last_sync >= d.last_sync => {}
            _ => {
                map.insert(&d.id, d);
            }
        }
    }
    let mut out: Vec<Device> = map
        .into_values()
        .filter(|d| now - d.last_sync <= DEVICE_MAX_AGE_MS)
        .cloned()
        .collect();
    out.sort_by(|x, y| x.id.cmp(&y.id));
    out
}

/// The result of merging this device's dictionary with the mailbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeOutcome {
    /// What this device should now store locally (tombstones included).
    pub entries: Vec<Entry>,
    /// What to upload, if an upload is needed.
    pub upload: Option<Payload>,
}

/// Merges local entries into the mailbox content. An upload is needed when
/// the merged entries differ from the mailbox, or this device's own record
/// is missing, renamed or older than the heartbeat.
pub fn merge_with_mailbox(local: &[Entry], remote: Option<&Payload>, me: &Device, now: i64) -> MergeOutcome {
    let empty = Payload::default();
    let remote = remote.unwrap_or(&empty);
    let mut entries = merge_entries(local, &remote.entries);
    entries.retain(|e| !(e.deleted && now - e.updated_at > TOMBSTONE_MAX_AGE_MS));
    let mine = remote.devices.iter().find(|d| d.id == me.id);
    let stale = match mine {
        None => true,
        Some(d) => d.name != me.name || d.kind != me.kind || now - d.last_sync >= HEARTBEAT_MS,
    };
    let mut remote_sorted = remote.entries.clone();
    remote_sorted.sort_by(|a, b| a.id.cmp(&b.id));
    let changed = entries != remote_sorted;
    let upload = (changed || stale).then(|| {
        let fresh = Device {
            last_sync: now,
            ..me.clone()
        };
        Payload {
            entries: entries.clone(),
            devices: merge_devices(&remote.devices, &[fresh], now),
        }
    });
    MergeOutcome { entries, upload }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, term: &str, at: i64) -> Entry {
        Entry {
            id: id.into(),
            term: term.into(),
            sounds_like: vec![],
            note: String::new(),
            updated_at: at,
            deleted: false,
        }
    }

    fn me(now: i64) -> Device {
        Device {
            id: "11111111-1111-4111-8111-111111111111".into(),
            name: "DESKTOP".into(),
            kind: DeviceKind::Computer,
            last_sync: now,
        }
    }

    const NOW: i64 = 1_800_000_000_000;

    fn vector_secret() -> String {
        let bytes: Vec<u8> = (0u8..32).collect();
        B64.encode(bytes)
    }

    /// Published in docs/sync-protocol.md. The Android app must match it.
    #[test]
    fn known_test_vector() {
        let secret = vector_secret();
        assert_eq!(secret, "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=");
        let k = derive(&secret).unwrap();
        assert_eq!(k.mailbox, VECTOR_MAILBOX);
        assert_eq!(k.token, VECTOR_TOKEN);
        assert_eq!(hex(&k.key), VECTOR_KEY);
        assert_eq!(token_hash(&k.token), VECTOR_TOKEN_HASH);
        let nonce: [u8; 12] = core::array::from_fn(|i| i as u8);
        let s = encrypt_with_nonce(&k, &nonce, br#"{"entries":[],"devices":[]}"#);
        assert_eq!(s.nonce, "AAECAwQFBgcICQoL");
        assert_eq!(s.ciphertext, VECTOR_CIPHERTEXT);
        assert_eq!(decrypt(&k, &s).unwrap(), br#"{"entries":[],"devices":[]}"#);
    }

    const VECTOR_MAILBOX: &str = "edaba0f8f5a6b41660b29dfc43e0c1f9c3067ab5d6cef6c3564a014f7752ee28";
    const VECTOR_TOKEN: &str = "AJz6SLfpq2jeFQJFKtPqDMJjLGr4HmRTVsv4YRMp_Cw";
    const VECTOR_KEY: &str = "beccb5b399d59b7385c4d5d5df2090be773deaf000144a2b2caa2a567232e841";
    const VECTOR_TOKEN_HASH: &str = "81d430e62ca3afa96592b7a7c50f6213ccbde0fd21a6cebfa6b0d5038facb16c";
    const VECTOR_CIPHERTEXT: &str = "UcqqKN+j0S2YVtLI2StUerwpqF/KNKjI1Hzcac50dzPtEw5tYv5gpse67A==";

    #[test]
    fn derive_rejects_bad_secrets() {
        assert_eq!(derive("not base64!"), Err(SyncError::BadSecret));
        assert_eq!(derive(&B64.encode([0u8; 16])), Err(SyncError::BadSecret));
        let k = derive(&new_secret()).unwrap();
        assert_eq!(k.mailbox.len(), 64);
        assert_eq!(k.token.len(), 43);
    }

    #[test]
    fn payload_roundtrip_and_wrong_secret() {
        let k = derive(&new_secret()).unwrap();
        let p = Payload {
            entries: vec![entry("a", "Sela", 5)],
            devices: vec![me(NOW)],
        };
        let s = seal_payload(&k, &p);
        assert_eq!(open_payload(&k, &s).unwrap(), p);
        let other = derive(&new_secret()).unwrap();
        assert_eq!(open_payload(&other, &s), Err(SyncError::Decrypt));
        let mut tampered = s.clone();
        tampered.ciphertext = B64.encode(b"0123456789abcdef0123");
        assert_eq!(open_payload(&k, &tampered), Err(SyncError::Decrypt));
    }

    #[test]
    fn first_sync_uploads_local_dictionary() {
        let out = merge_with_mailbox(&[entry("a", "Sela", 5)], None, &me(NOW), NOW);
        let up = out.upload.unwrap();
        assert_eq!(up.entries.len(), 1);
        assert_eq!(up.devices, vec![me(NOW)]);
    }

    #[test]
    fn nothing_to_upload_when_in_step_and_recent() {
        let remote = Payload {
            entries: vec![entry("a", "Sela", 5)],
            devices: vec![me(NOW - 1000)],
        };
        let out = merge_with_mailbox(&[entry("a", "Sela", 5)], Some(&remote), &me(NOW), NOW);
        assert!(out.upload.is_none());
        assert_eq!(out.entries, remote.entries);
    }

    #[test]
    fn heartbeat_uploads_after_15_minutes() {
        let remote = Payload {
            entries: vec![],
            devices: vec![me(NOW - HEARTBEAT_MS)],
        };
        assert!(merge_with_mailbox(&[], Some(&remote), &me(NOW), NOW).upload.is_some());
    }

    #[test]
    fn remote_changes_come_down_and_local_changes_go_up() {
        let phone = Device {
            id: "22222222-2222-4222-8222-222222222222".into(),
            name: "Pixel".into(),
            kind: DeviceKind::Phone,
            last_sync: NOW - 5000,
        };
        let remote = Payload {
            entries: vec![entry("a", "Sela", 10), entry("b", "phone term", 3)],
            devices: vec![phone.clone(), me(NOW - 1000)],
        };
        let local = vec![entry("a", "Selah", 5), entry("c", "desk term", 4)];
        let out = merge_with_mailbox(&local, Some(&remote), &me(NOW), NOW);
        let terms: Vec<&str> = out.entries.iter().map(|e| e.term.as_str()).collect();
        assert_eq!(
            terms,
            vec!["Sela", "phone term", "desk term"],
            "newer remote edit wins, both new terms kept"
        );
        let up = out.upload.unwrap();
        assert_eq!(up.entries, out.entries);
        assert_eq!(up.devices.len(), 2);
        assert!(up.devices.contains(&phone));
    }

    #[test]
    fn old_tombstones_and_devices_are_dropped() {
        let mut t = entry("t", "gone", 1);
        t.deleted = true;
        let ghost = Device {
            id: "33333333-3333-4333-8333-333333333333".into(),
            name: "Old phone".into(),
            kind: DeviceKind::Phone,
            last_sync: NOW - DEVICE_MAX_AGE_MS - 1,
        };
        let remote = Payload {
            entries: vec![t],
            devices: vec![ghost],
        };
        let out = merge_with_mailbox(&[], Some(&remote), &me(NOW), NOW);
        assert!(out.entries.is_empty());
        assert_eq!(out.upload.unwrap().devices, vec![me(NOW)]);
    }

    #[test]
    fn device_merge_keeps_newest() {
        let a = Device { last_sync: 5, ..me(0) };
        let b = Device {
            last_sync: 9,
            name: "Renamed".into(),
            ..me(0)
        };
        assert_eq!(
            merge_devices(std::slice::from_ref(&a), std::slice::from_ref(&b), 10),
            vec![b.clone()]
        );
        assert_eq!(merge_devices(std::slice::from_ref(&b), &[a], 10), vec![b]);
    }

    #[test]
    fn qr_json_shape() {
        let qr = PairingQr {
            v: PROTOCOL_VERSION,
            name: "DESKTOP".into(),
            relay: DEFAULT_RELAY.into(),
            secret: vector_secret(),
        };
        let json = serde_json::to_string(&qr).unwrap();
        assert_eq!(
            json,
            r#"{"v":2,"name":"DESKTOP","relay":"https://wirl-relay.wirl.workers.dev","secret":"AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="}"#
        );
    }
}
