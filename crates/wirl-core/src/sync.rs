//! Phone sync over Wi-Fi. See docs/sync-protocol.md; the Android app
//! implements the same protocol.

use crate::dictionary::{merge_entries, Entry};
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use hkdf::Hkdf;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

pub const PROTOCOL_VERSION: u32 = 1;
pub const SERVICE_TYPE: &str = "_wirl._tcp.local.";
pub const SYNC_PATH: &str = "/wirl/sync";
pub const HKDF_INFO: &[u8] = b"wirl-sync-v1";
pub const DEFAULT_PORT: u16 = 47913;

/// Contents of the pairing QR code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingQr {
    pub v: u32,
    pub device_id: String,
    pub name: String,
    pub secret: String,
    pub port: u16,
    pub hosts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRequest {
    pub v: u32,
    pub phone_id: String,
    pub phone_name: String,
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResponse {
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntriesPayload {
    pub entries: Vec<Entry>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SyncError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("bad request: {0}")]
    BadRequest(String),
}

impl SyncError {
    pub fn http_status(&self) -> u16 {
        match self {
            SyncError::Unauthorized => 401,
            SyncError::BadRequest(_) => 400,
        }
    }
}

/// 32 random bytes, base64 encoded.
pub fn new_secret() -> String {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    B64.encode(b)
}

/// HKDF-SHA256(ikm = decoded secret bytes, salt = none, info = "wirl-sync-v1") -> 32 bytes.
pub fn derive_key(secret_b64: &str) -> Result<[u8; 32], SyncError> {
    let ikm = B64
        .decode(secret_b64.trim())
        .map_err(|_| SyncError::BadRequest("secret is not base64".into()))?;
    let hk = Hkdf::<Sha256>::new(None, &ikm);
    let mut okm = [0u8; 32];
    hk.expand(HKDF_INFO, &mut okm)
        .map_err(|_| SyncError::BadRequest("hkdf".into()))?;
    Ok(okm)
}

/// AES-256-GCM encrypt with AAD = phoneId. Returns (nonce_b64, ciphertext_b64);
/// the ciphertext carries the 16-byte tag appended, as is standard.
pub fn encrypt(key: &[u8; 32], phone_id: &str, plaintext: &[u8]) -> (String, String) {
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    (B64.encode(nonce), encrypt_with_nonce(key, phone_id, &nonce, plaintext))
}

/// Deterministic form, for test vectors only. Never reuse a nonce.
pub fn encrypt_with_nonce(key: &[u8; 32], phone_id: &str, nonce: &[u8; 12], plaintext: &[u8]) -> String {
    let cipher = Aes256Gcm::new(key.into());
    let ct = cipher
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad: phone_id.as_bytes(),
            },
        )
        .expect("aes-gcm encryption cannot fail for in-memory buffers");
    B64.encode(ct)
}

pub fn decrypt(key: &[u8; 32], phone_id: &str, nonce_b64: &str, ct_b64: &str) -> Result<Vec<u8>, SyncError> {
    let nonce = B64.decode(nonce_b64).map_err(|_| SyncError::Unauthorized)?;
    if nonce.len() != 12 {
        return Err(SyncError::Unauthorized);
    }
    let ct = B64.decode(ct_b64).map_err(|_| SyncError::Unauthorized)?;
    let cipher = Aes256Gcm::new(key.into());
    cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &ct,
                aad: phone_id.as_bytes(),
            },
        )
        .map_err(|_| SyncError::Unauthorized)
}

/// Result of a successful sync exchange.
#[derive(Debug)]
pub struct SyncOutcome {
    pub phone_id: String,
    pub phone_name: String,
    /// Which candidate secret worked (index into the list passed in).
    pub secret_index: usize,
    pub merged: Vec<Entry>,
    pub response: SyncResponse,
}

/// Handles one POST /wirl/sync body. `candidate_secrets` are the secrets
/// to try for this phone: its stored secret if paired, plus any pending
/// pairing secret. Decryption failure with all of them means 401.
pub fn handle_sync(
    body: &[u8],
    candidate_secrets: &dyn Fn(&str) -> Vec<String>,
    local: &[Entry],
) -> Result<SyncOutcome, SyncError> {
    let req: SyncRequest = serde_json::from_slice(body).map_err(|e| SyncError::BadRequest(e.to_string()))?;
    if req.v != PROTOCOL_VERSION {
        return Err(SyncError::BadRequest(format!("unsupported version {}", req.v)));
    }
    if uuid::Uuid::parse_str(&req.phone_id).is_err() {
        return Err(SyncError::BadRequest("phoneId must be a UUID".into()));
    }
    let secrets = candidate_secrets(&req.phone_id);
    for (i, secret) in secrets.iter().enumerate() {
        let Ok(key) = derive_key(secret) else { continue };
        let Ok(plain) = decrypt(&key, &req.phone_id, &req.nonce, &req.ciphertext) else {
            continue;
        };
        let payload: EntriesPayload =
            serde_json::from_slice(&plain).map_err(|e| SyncError::BadRequest(format!("payload: {e}")))?;
        let mut merged = merge_entries(local, &payload.entries);
        let now = crate::dictionary::now_ms();
        merged.retain(|e| !(e.deleted && now - e.updated_at > crate::dictionary::TOMBSTONE_MAX_AGE_MS));
        let out = serde_json::to_vec(&EntriesPayload {
            entries: merged.clone(),
        })
        .expect("serialize");
        let (nonce, ciphertext) = encrypt(&key, &req.phone_id, &out);
        return Ok(SyncOutcome {
            phone_id: req.phone_id,
            phone_name: req.phone_name,
            secret_index: i,
            merged,
            response: SyncResponse { nonce, ciphertext },
        });
    }
    Err(SyncError::Unauthorized)
}

/// Builds a request the way the phone does. Used by tests and the
/// developer diagnostics "simulate phone" tool.
pub fn build_phone_request(secret: &str, phone_id: &str, phone_name: &str, entries: &[Entry]) -> Vec<u8> {
    let key = derive_key(secret).expect("valid secret");
    let plain = serde_json::to_vec(&EntriesPayload {
        entries: entries.to_vec(),
    })
    .unwrap();
    let (nonce, ciphertext) = encrypt(&key, phone_id, &plain);
    serde_json::to_vec(&SyncRequest {
        v: PROTOCOL_VERSION,
        phone_id: phone_id.into(),
        phone_name: phone_name.into(),
        nonce,
        ciphertext,
    })
    .unwrap()
}

/// True for RFC 1918, link-local and loopback IPv4 addresses.
pub fn is_private_ipv4(ip: std::net::Ipv4Addr) -> bool {
    ip.is_private() || ip.is_link_local() || ip.is_loopback()
}

/// True if a peer address may talk to the sync server.
pub fn is_allowed_peer(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => is_private_ipv4(v4),
        std::net::IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private_ipv4(v4);
            }
            v6.is_loopback() || (v6.segments()[0] & 0xffc0) == 0xfe80 || (v6.segments()[0] & 0xfe00) == 0xfc00
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, term: &str, at: i64) -> Entry {
        Entry {
            id: id.into(),
            term: term.into(),
            sounds_like: vec!["x".into()],
            note: "".into(),
            updated_at: at,
            deleted: false,
        }
    }

    const PHONE: &str = "0b7c6f7e-1d2a-4b5c-9d8e-7f6a5b4c3d2e";

    #[test]
    fn encrypt_decrypt_roundtrip_with_aad() {
        let key = derive_key(&new_secret()).unwrap();
        let (n, c) = encrypt(&key, PHONE, b"hello");
        assert_eq!(decrypt(&key, PHONE, &n, &c).unwrap(), b"hello");
        // Wrong AAD (phoneId) fails.
        assert_eq!(decrypt(&key, "other", &n, &c), Err(SyncError::Unauthorized));
    }

    /// Published in docs/sync-protocol.md; cross-checked against Python's
    /// `cryptography` package. The Android app should pass the same vector.
    #[test]
    fn known_test_vector() {
        let secret_bytes: Vec<u8> = (0u8..32).collect();
        let secret = B64.encode(&secret_bytes);
        assert_eq!(secret, "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=");
        let key = derive_key(&secret).unwrap();
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "f2b4a1a6ce55a07aea8ccb22e73d9ad6f36e4e177601c4e0d1be7ad47e6eb66c");
        let nonce: [u8; 12] = core::array::from_fn(|i| i as u8);
        let ct = encrypt_with_nonce(&key, PHONE, &nonce, br#"{"entries":[]}"#);
        assert_eq!(ct, "T4GGLLyKJaiy63rNMucjUgABVlyT/DGPB7LUjk1Z");
        assert_eq!(
            decrypt(&key, PHONE, &B64.encode(nonce), &ct).unwrap(),
            br#"{"entries":[]}"#
        );
    }

    #[test]
    fn full_exchange_merges_and_both_sides_match() {
        let secret = new_secret();
        let local = vec![entry("a", "desk", 5), entry("b", "old", 1)];
        let phone = vec![entry("b", "new", 2), entry("c", "phone", 3)];
        let body = build_phone_request(&secret, PHONE, "Pixel", &phone);
        let s2 = secret.clone();
        let out = handle_sync(&body, &move |_| vec![s2.clone()], &local).unwrap();
        assert_eq!(out.phone_name, "Pixel");
        assert_eq!(out.merged.len(), 3);
        // Phone decrypts the response and gets exactly the merged list.
        let key = derive_key(&secret).unwrap();
        let plain = decrypt(&key, PHONE, &out.response.nonce, &out.response.ciphertext).unwrap();
        let p: EntriesPayload = serde_json::from_slice(&plain).unwrap();
        assert_eq!(p.entries, out.merged);
        assert_eq!(merge_entries(&phone, &p.entries), out.merged);
    }

    #[test]
    fn old_tombstones_are_purged_in_sync() {
        let secret = new_secret();
        let mut old = entry("t", "gone", 1);
        old.deleted = true;
        let body = build_phone_request(&secret, PHONE, "Pixel", &[old]);
        let out = handle_sync(&body, &move |_| vec![secret.clone()], &[]).unwrap();
        assert!(out.merged.is_empty());
    }

    #[test]
    fn wrong_secret_is_rejected_with_401() {
        let body = build_phone_request(&new_secret(), PHONE, "Pixel", &[]);
        let wrong = new_secret();
        let err = handle_sync(&body, &move |_| vec![wrong.clone()], &[]).unwrap_err();
        assert_eq!(err, SyncError::Unauthorized);
        assert_eq!(err.http_status(), 401);
        let err = handle_sync(&body, &|_| vec![], &[]).unwrap_err();
        assert_eq!(err.http_status(), 401);
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let secret = new_secret();
        let body = build_phone_request(&secret, PHONE, "Pixel", &[]);
        let mut req: SyncRequest = serde_json::from_slice(&body).unwrap();
        let mut ct = B64.decode(&req.ciphertext).unwrap();
        ct[0] ^= 1;
        req.ciphertext = B64.encode(ct);
        let body = serde_json::to_vec(&req).unwrap();
        let err = handle_sync(&body, &move |_| vec![secret.clone()], &[]).unwrap_err();
        assert_eq!(err.http_status(), 401);
    }

    #[test]
    fn second_candidate_secret_reports_index() {
        let pending = new_secret();
        let body = build_phone_request(&pending, PHONE, "Pixel", &[]);
        let list = vec![new_secret(), pending];
        let out = handle_sync(&body, &move |_| list.clone(), &[]).unwrap();
        assert_eq!(out.secret_index, 1);
    }

    #[test]
    fn qr_json_shape() {
        let qr = PairingQr {
            v: 1,
            device_id: "id".into(),
            name: "PC".into(),
            secret: "s".into(),
            port: 47913,
            hosts: vec!["192.168.1.5".into()],
        };
        let s = serde_json::to_string(&qr).unwrap();
        assert_eq!(
            s,
            r#"{"v":1,"deviceId":"id","name":"PC","secret":"s","port":47913,"hosts":["192.168.1.5"]}"#
        );
    }

    #[test]
    fn private_peers_only() {
        use std::net::{IpAddr, Ipv4Addr};
        assert!(is_allowed_peer(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2))));
        assert!(is_allowed_peer(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2))));
        assert!(is_allowed_peer(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 2))));
        assert!(!is_allowed_peer(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }
}
