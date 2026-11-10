//! Client for the mailbox relay (docs/sync-protocol.md §5) and the sync
//! round: fetch, merge, upload with compare-and-swap, retry on conflict.

use crate::dictionary::Entry;
use crate::sync::{merge_with_mailbox, open_payload, seal_payload, Device, GroupKeys, Sealed, SyncError};
use serde::{Deserialize, Serialize};
use std::future::Future;
#[cfg(feature = "net")]
use std::time::Duration;

/// Uploads retried this many times when another device wrote first.
pub const MAX_CONFLICT_RETRIES: usize = 5;
/// Largest request the relay accepts.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RelayError {
    /// 401: the mailbox belongs to a different secret.
    #[error("this device's pairing is no longer valid")]
    NeedsPairing,
    /// 410: the computer disconnected its phones (the mailbox was closed).
    #[error("this sync group was closed")]
    Closed,
    #[error("couldn't reach the sync service: {0}")]
    Network(String),
    #[error("the sync service answered {0}")]
    Server(u16),
    #[error("{0}")]
    Data(#[from] SyncError),
    #[error("kept conflicting with another device; will retry")]
    TooManyConflicts,
}

impl RelayError {
    /// The user has to pair again (scan a new code).
    pub fn needs_pairing(&self) -> bool {
        matches!(
            self,
            RelayError::NeedsPairing | RelayError::Closed | RelayError::Data(SyncError::Decrypt)
        )
    }

    /// A short, plain message for the UI.
    pub fn plain(&self) -> String {
        match self {
            RelayError::NeedsPairing | RelayError::Closed | RelayError::Data(SyncError::Decrypt) => {
                "Pairing is no longer valid. Pair again.".into()
            }
            RelayError::Network(_) => "Can't reach the sync service. Will try again.".into(),
            RelayError::Server(_) | RelayError::TooManyConflicts => "Sync didn't finish. Will try again.".into(),
            RelayError::Data(_) => "The synced dictionary couldn't be read.".into(),
        }
    }
}

/// The mailbox's current value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remote {
    pub version: u64,
    pub nonce: String,
    pub ciphertext: String,
}

impl Remote {
    pub fn sealed(&self) -> Sealed {
        Sealed {
            nonce: self.nonce.clone(),
            ciphertext: self.ciphertext.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PutResult {
    Stored {
        version: u64,
    },
    /// Someone else wrote first; this is the current value.
    Conflict(Remote),
}

/// The three relay calls. `HttpRelay` is the real one; tests use a fake.
pub trait Transport {
    fn get(&self, keys: &GroupKeys) -> impl Future<Output = Result<Option<Remote>, RelayError>> + Send;
    fn put(
        &self,
        keys: &GroupKeys,
        base_version: u64,
        sealed: &Sealed,
    ) -> impl Future<Output = Result<PutResult, RelayError>> + Send;
    fn close(&self, keys: &GroupKeys) -> impl Future<Output = Result<(), RelayError>> + Send;
}

/// What a sync round produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncOutcome {
    /// The dictionary to keep locally, tombstones included.
    pub entries: Vec<Entry>,
    /// Everyone in the group, this device included.
    pub devices: Vec<Device>,
    pub uploaded: bool,
}

/// One sync round: fetch, merge, upload if needed, retry on conflicts.
pub async fn sync_once<T: Transport>(
    t: &T,
    keys: &GroupKeys,
    local: &[Entry],
    me: &Device,
    now: i64,
) -> Result<SyncOutcome, RelayError> {
    let mut remote = t.get(keys).await?;
    for _ in 0..=MAX_CONFLICT_RETRIES {
        let (version, payload) = match &remote {
            Some(r) => (r.version, Some(open_payload(keys, &r.sealed())?)),
            None => (0, None),
        };
        let outcome = merge_with_mailbox(local, payload.as_ref(), me, now);
        let Some(upload) = outcome.upload else {
            let devices = payload.map(|p| p.devices).unwrap_or_default();
            return Ok(SyncOutcome {
                entries: outcome.entries,
                devices,
                uploaded: false,
            });
        };
        match t.put(keys, version, &seal_payload(keys, &upload)).await? {
            PutResult::Stored { .. } => {
                return Ok(SyncOutcome {
                    entries: outcome.entries,
                    devices: upload.devices,
                    uploaded: true,
                })
            }
            PutResult::Conflict(current) => remote = Some(current),
        }
    }
    Err(RelayError::TooManyConflicts)
}

/// The real relay over HTTPS.
#[cfg(feature = "net")]
pub struct HttpRelay {
    pub client: reqwest::Client,
    /// Base URL, e.g. "https://wirl-relay.wirl.workers.dev".
    pub base: String,
}

#[cfg(feature = "net")]
impl HttpRelay {
    pub fn new(base: &str) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        HttpRelay {
            client,
            base: base.trim_end_matches('/').to_string(),
        }
    }

    fn url(&self, keys: &GroupKeys) -> String {
        format!("{}/v2/m/{}", self.base, keys.mailbox)
    }
}

#[cfg(feature = "net")]
fn net(e: reqwest::Error) -> RelayError {
    RelayError::Network(e.to_string())
}

#[cfg(feature = "net")]
fn status_error(code: u16) -> RelayError {
    match code {
        401 => RelayError::NeedsPairing,
        410 => RelayError::Closed,
        c => RelayError::Server(c),
    }
}

#[cfg(feature = "net")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PutBody<'a> {
    base_version: u64,
    nonce: &'a str,
    ciphertext: &'a str,
}

#[cfg(feature = "net")]
#[derive(Deserialize)]
struct Stored {
    version: u64,
}

#[cfg(feature = "net")]
impl Transport for HttpRelay {
    async fn get(&self, keys: &GroupKeys) -> Result<Option<Remote>, RelayError> {
        let r = self
            .client
            .get(self.url(keys))
            .bearer_auth(&keys.token)
            .send()
            .await
            .map_err(net)?;
        match r.status().as_u16() {
            200 => Ok(Some(r.json::<Remote>().await.map_err(net)?)),
            404 => Ok(None),
            c => Err(status_error(c)),
        }
    }

    async fn put(&self, keys: &GroupKeys, base_version: u64, sealed: &Sealed) -> Result<PutResult, RelayError> {
        let body = PutBody {
            base_version,
            nonce: &sealed.nonce,
            ciphertext: &sealed.ciphertext,
        };
        let r = self
            .client
            .put(self.url(keys))
            .bearer_auth(&keys.token)
            .json(&body)
            .send()
            .await
            .map_err(net)?;
        match r.status().as_u16() {
            200 => Ok(PutResult::Stored {
                version: r.json::<Stored>().await.map_err(net)?.version,
            }),
            409 => Ok(PutResult::Conflict(r.json::<Remote>().await.map_err(net)?)),
            c => Err(status_error(c)),
        }
    }

    async fn close(&self, keys: &GroupKeys) -> Result<(), RelayError> {
        let r = self
            .client
            .delete(self.url(keys))
            .bearer_auth(&keys.token)
            .send()
            .await
            .map_err(net)?;
        match r.status().as_u16() {
            200 | 204 | 404 | 410 => Ok(()),
            c => Err(status_error(c)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{derive, new_secret, DeviceKind, Payload};
    use std::sync::Mutex;

    /// An in-memory relay with the same rules as relay/src/mailbox.js.
    #[derive(Default)]
    struct Fake {
        state: Mutex<Option<Remote>>,
        closed: Mutex<bool>,
        /// Before the next PUT is checked, another device writes this payload.
        race: Mutex<Option<Payload>>,
        puts: Mutex<usize>,
    }

    impl Transport for Fake {
        async fn get(&self, _: &GroupKeys) -> Result<Option<Remote>, RelayError> {
            if *self.closed.lock().unwrap() {
                return Err(RelayError::Closed);
            }
            Ok(self.state.lock().unwrap().clone())
        }
        async fn put(&self, keys: &GroupKeys, base: u64, sealed: &Sealed) -> Result<PutResult, RelayError> {
            *self.puts.lock().unwrap() += 1;
            let mut st = self.state.lock().unwrap();
            if let Some(p) = self.race.lock().unwrap().take() {
                let s = crate::sync::seal_payload(keys, &p);
                let v = st.as_ref().map(|r| r.version).unwrap_or(0) + 1;
                *st = Some(Remote {
                    version: v,
                    nonce: s.nonce,
                    ciphertext: s.ciphertext,
                });
            }
            let cur = st.as_ref().map(|r| r.version).unwrap_or(0);
            if cur != base {
                return Ok(PutResult::Conflict(st.clone().unwrap()));
            }
            *st = Some(Remote {
                version: cur + 1,
                nonce: sealed.nonce.clone(),
                ciphertext: sealed.ciphertext.clone(),
            });
            Ok(PutResult::Stored { version: cur + 1 })
        }
        async fn close(&self, _: &GroupKeys) -> Result<(), RelayError> {
            *self.closed.lock().unwrap() = true;
            Ok(())
        }
    }

    fn e(id: &str, term: &str, at: i64) -> Entry {
        Entry {
            id: id.into(),
            term: term.into(),
            sounds_like: vec![],
            note: String::new(),
            updated_at: at,
            deleted: false,
        }
    }

    fn dev(id: &str, kind: DeviceKind) -> Device {
        Device {
            id: id.into(),
            name: id.into(),
            kind,
            last_sync: 0,
        }
    }

    const NOW: i64 = 1_800_000_000_000;

    #[tokio::test]
    async fn two_devices_end_up_with_the_same_dictionary() {
        let keys = derive(&new_secret()).unwrap();
        let relay = Fake::default();
        let pc = dev("pc", DeviceKind::Computer);
        let phone = dev("phone", DeviceKind::Phone);
        let pc_local = vec![e("a", "Sela", 5)];
        let first = sync_once(&relay, &keys, &pc_local, &pc, NOW).await.unwrap();
        assert!(first.uploaded);
        let phone_local = vec![e("b", "Bina", 6)];
        let second = sync_once(&relay, &keys, &phone_local, &phone, NOW + 1).await.unwrap();
        assert_eq!(second.entries.len(), 2);
        assert_eq!(second.devices.len(), 2);
        let third = sync_once(&relay, &keys, &first.entries, &pc, NOW + 2).await.unwrap();
        assert_eq!(third.entries, second.entries, "the computer picks up the phone's term");
    }

    #[tokio::test]
    async fn conflict_is_merged_and_retried() {
        let keys = derive(&new_secret()).unwrap();
        let relay = Fake::default();
        // While the computer uploads, the phone writes its own term first.
        *relay.race.lock().unwrap() = Some(Payload {
            entries: vec![e("p", "from phone", 7)],
            devices: vec![],
        });
        let pc = dev("pc", DeviceKind::Computer);
        let out = sync_once(&relay, &keys, &[e("c", "from pc", 8)], &pc, NOW)
            .await
            .unwrap();
        let terms: Vec<&str> = out.entries.iter().map(|x| x.term.as_str()).collect();
        assert_eq!(terms, vec!["from pc", "from phone"]);
        assert_eq!(*relay.puts.lock().unwrap(), 2, "one conflict, one retry");
        let stored = relay.state.lock().unwrap().clone().unwrap();
        assert_eq!(stored.version, 2);
        assert_eq!(open_payload(&keys, &stored.sealed()).unwrap().entries.len(), 2);
    }

    #[tokio::test]
    async fn no_upload_when_nothing_changed() {
        let keys = derive(&new_secret()).unwrap();
        let relay = Fake::default();
        let pc = dev("pc", DeviceKind::Computer);
        let local = vec![e("a", "Sela", 5)];
        sync_once(&relay, &keys, &local, &pc, NOW).await.unwrap();
        let again = sync_once(&relay, &keys, &local, &pc, NOW + 1000).await.unwrap();
        assert!(!again.uploaded);
        assert_eq!(*relay.puts.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn closed_mailbox_means_pair_again() {
        let keys = derive(&new_secret()).unwrap();
        let relay = Fake::default();
        relay.close(&keys).await.unwrap();
        let err = sync_once(&relay, &keys, &[], &dev("phone", DeviceKind::Phone), NOW)
            .await
            .unwrap_err();
        assert!(err.needs_pairing());
        assert_eq!(err.plain(), "Pairing is no longer valid. Pair again.");
    }

    #[tokio::test]
    async fn wrong_secret_cannot_read_the_mailbox() {
        let keys = derive(&new_secret()).unwrap();
        let relay = Fake::default();
        sync_once(
            &relay,
            &keys,
            &[e("a", "Sela", 5)],
            &dev("pc", DeviceKind::Computer),
            NOW,
        )
        .await
        .unwrap();
        let other = derive(&new_secret()).unwrap();
        let err = sync_once(&relay, &other, &[], &dev("x", DeviceKind::Phone), NOW)
            .await
            .unwrap_err();
        assert_eq!(err, RelayError::Data(SyncError::Decrypt));
        assert!(err.needs_pairing());
    }
}
