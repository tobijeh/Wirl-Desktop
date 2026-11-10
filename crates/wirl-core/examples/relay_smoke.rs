//! End-to-end check against a running relay (manual, not in CI):
//!   cd relay && npx wrangler@4 dev --local --port 8799
//!   WIRL_RELAY_URL=http://127.0.0.1:8799 cargo run -p wirl-core --example relay_smoke
//! Two simulated devices sync through the relay and must end up identical.

use wirl_core::dictionary::{now_ms, Entry};
use wirl_core::relay::{sync_once, HttpRelay, Transport};
use wirl_core::sync::{derive, new_secret, Device, DeviceKind};

fn device(id: &str, kind: DeviceKind) -> Device {
    Device {
        id: id.into(),
        name: id.into(),
        kind,
        last_sync: 0,
    }
}

#[tokio::main]
async fn main() {
    let base = std::env::var("WIRL_RELAY_URL").expect("set WIRL_RELAY_URL");
    let relay = HttpRelay::new(&base);
    let keys = derive(&new_secret()).unwrap();
    let pc = device("computer", DeviceKind::Computer);
    let phone = device("phone", DeviceKind::Phone);

    let pc_local = vec![Entry::new("Sela", vec!["salad".into()], "daughter")];
    let a = sync_once(&relay, &keys, &pc_local, &pc, now_ms())
        .await
        .expect("computer sync");
    println!("computer: uploaded={} entries={}", a.uploaded, a.entries.len());

    let phone_local = vec![Entry::new("Bina", vec![], "")];
    let b = sync_once(&relay, &keys, &phone_local, &phone, now_ms())
        .await
        .expect("phone sync");
    println!(
        "phone: uploaded={} entries={} devices={}",
        b.uploaded,
        b.entries.len(),
        b.devices.len()
    );

    let c = sync_once(&relay, &keys, &a.entries, &pc, now_ms())
        .await
        .expect("computer again");
    println!("computer again: uploaded={} entries={}", c.uploaded, c.entries.len());
    assert_eq!(c.entries, b.entries, "both devices hold the same dictionary");

    relay.close(&keys).await.expect("close");
    let closed = sync_once(&relay, &keys, &[], &phone, now_ms()).await.unwrap_err();
    println!("after close: {closed} (needs pairing: {})", closed.needs_pairing());
    assert!(closed.needs_pairing());
    println!("OK");
}
