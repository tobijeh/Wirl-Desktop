//! Phone sync server over Wi-Fi. Protocol: docs/sync-protocol.md.
//! Listens on all interfaces but only answers private-network peers, and
//! advertises itself over mDNS as `_wirl._tcp`.

use crate::app::{self, PendingPairing, PAIRING_TTL_MS};
use crate::storage::{self, PairedPhone};
use axum::body::Bytes;
use axum::extract::ConnectInfo;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;
use wirl_core::diagnostics::{DiagEntry, DiagKind};
use wirl_core::dictionary::now_ms;
use wirl_core::sync::{self, PairingQr, DEFAULT_PORT, SERVICE_TYPE, SYNC_PATH};

const MAX_BODY: usize = 8 * 1024 * 1024;

/// This computer's private IPv4 addresses (for the QR code and mDNS).
pub fn private_ipv4s() -> Vec<Ipv4Addr> {
    let mut v: Vec<Ipv4Addr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(ip) if ip.is_private() && !i.is_loopback() => Some(ip),
            _ => None,
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

pub fn computer_name() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "Wirl computer".into())
}

async fn handle(ConnectInfo(peer): ConnectInfo<SocketAddr>, body: Bytes) -> impl IntoResponse {
    if !sync::is_allowed_peer(peer.ip()) {
        return (StatusCode::FORBIDDEN, String::new());
    }
    if body.len() > MAX_BODY {
        return (StatusCode::PAYLOAD_TOO_LARGE, String::new());
    }
    let result = tokio::task::spawn_blocking(move || process(&body)).await;
    match result {
        Ok(Ok(json)) => (StatusCode::OK, json),
        Ok(Err(e)) => (
            StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
            String::new(),
        ),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
    }
}

fn process(body: &[u8]) -> Result<String, sync::SyncError> {
    let a = app::get();
    let pending = a
        .pairing
        .lock()
        .clone()
        .filter(|p| now_ms() - p.created_ms < PAIRING_TTL_MS);
    let candidates = |phone_id: &str| {
        let mut v = Vec::new();
        if let Some(s) = storage::get_phone_secret(phone_id) {
            v.push(s);
        }
        if let Some(p) = &pending {
            v.push(p.secret.clone());
        }
        v
    };
    let local = a.dict.lock().entries.clone();
    let out = match sync::handle_sync(body, &candidates, &local) {
        Ok(o) => o,
        Err(e) => {
            if e == sync::SyncError::Unauthorized {
                a.log_diag(DiagEntry::new(DiagKind::Sync, "Rejected sync request (wrong secret)"));
            }
            return Err(e);
        }
    };
    // Candidates are [stored secret, pending secret]; which one matched
    // tells us whether this request completes a (re-)pairing.
    let used_pending = match storage::get_phone_secret(&out.phone_id) {
        Some(_) => out.secret_index == 1,
        None => true,
    };
    if used_pending {
        if let Some(p) = &pending {
            if storage::set_phone_secret(&out.phone_id, &p.secret).is_ok() {
                *a.pairing.lock() = None;
            }
        }
    }
    {
        let mut dev = a.device.lock();
        let now = now_ms();
        match dev.phones.iter_mut().find(|p| p.phone_id == out.phone_id) {
            Some(p) => {
                p.name = out.phone_name.clone();
                p.last_sync = Some(now);
            }
            None => dev.phones.push(PairedPhone {
                phone_id: out.phone_id.clone(),
                name: out.phone_name.clone(),
                paired_at: now,
                last_sync: Some(now),
            }),
        }
    }
    a.save_device();
    let before = a.dict.lock().count();
    {
        let mut d = a.dict.lock();
        d.entries = out.merged.clone();
    }
    a.save_dictionary();
    let after = a.dict.lock().count();
    a.log_diag(DiagEntry::new(
        DiagKind::Sync,
        format!("Synced with {} ({} terms, was {})", out.phone_name, after, before),
    ));
    if used_pending {
        let _ = tauri::Emitter::emit(&a.handle, "wirl://paired", out.phone_name.clone());
    }
    serde_json::to_string(&out.response).map_err(|e| sync::SyncError::BadRequest(e.to_string()))
}

/// Starts the server (first free port from 47913) and mDNS advertising.
pub fn start() {
    tauri::async_runtime::spawn(async {
        let mut listener = None;
        for port in DEFAULT_PORT..DEFAULT_PORT + 20 {
            if let Ok(l) = tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port))).await {
                listener = Some((l, port));
                break;
            }
        }
        let Some((listener, port)) = listener else {
            log::error!("sync: no free port");
            return;
        };
        *app::get().sync_port.lock() = Some(port);
        log::info!("sync server on port {port}");
        advertise(port);
        let router = Router::new().route(SYNC_PATH, post(handle));
        if let Err(e) = axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await {
            log::error!("sync server: {e}");
        }
    });
}

fn advertise(port: u16) {
    std::thread::spawn(move || {
        let daemon = match mdns_sd::ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                log::warn!("mDNS unavailable: {e}");
                return;
            }
        };
        let device_id = app::get().device.lock().device_id.clone();
        let name = computer_name();
        let host = format!("{}.local.", name.replace(' ', "-"));
        let mut last: Vec<Ipv4Addr> = Vec::new();
        loop {
            let ips = private_ipv4s();
            if ips != last && !ips.is_empty() {
                let addrs: Vec<std::net::IpAddr> = ips.iter().map(|i| std::net::IpAddr::V4(*i)).collect();
                let props = [("id", device_id.as_str())];
                match mdns_sd::ServiceInfo::new(SERVICE_TYPE, &name, &host, &addrs[..], port, &props[..]) {
                    Ok(info) => {
                        if let Err(e) = daemon.register(info) {
                            log::warn!("mDNS register: {e}");
                        }
                    }
                    Err(e) => log::warn!("mDNS service: {e}"),
                }
                last = ips;
            }
            std::thread::sleep(Duration::from_secs(120));
        }
    });
}

/// Starts pairing: new secret, QR JSON and SVG.
pub fn begin_pairing() -> Result<(String, String), String> {
    let a = app::get();
    let port = a
        .sync_port
        .lock()
        .ok_or("Sync isn't running yet. Try again in a moment.")?;
    let secret = sync::new_secret();
    *a.pairing.lock() = Some(PendingPairing {
        secret: secret.clone(),
        created_ms: now_ms(),
    });
    let hosts: Vec<String> = private_ipv4s().iter().map(|i| i.to_string()).collect();
    if hosts.is_empty() {
        return Err("Connect this computer to Wi-Fi to pair.".into());
    }
    let qr = PairingQr {
        v: sync::PROTOCOL_VERSION,
        device_id: a.device.lock().device_id.clone(),
        name: computer_name(),
        secret,
        port,
        hosts,
    };
    let json = serde_json::to_string(&qr).map_err(|e| e.to_string())?;
    let code = qrcode::QrCode::new(json.as_bytes()).map_err(|e| e.to_string())?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(240, 240)
        .quiet_zone(true)
        .dark_color(qrcode::render::svg::Color("#111116"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build();
    Ok((json, svg))
}

pub fn unpair(phone_id: &str) {
    let a = app::get();
    storage::delete_phone_secret(phone_id);
    a.device.lock().phones.retain(|p| p.phone_id != phone_id);
    a.save_device();
    a.log_diag(DiagEntry::new(DiagKind::Sync, "Unpaired a phone"));
}
