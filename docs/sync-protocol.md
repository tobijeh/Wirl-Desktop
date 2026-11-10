# Wirl dictionary sync protocol (v2: encrypted mailbox)

Wirl for Windows and Wirl for Android keep the personal dictionary in sync through a small **mailbox relay** on the internet. There are no accounts and no logins: scanning the computer's QR code is the pairing. Everything is encrypted on the devices before it leaves them, so the relay only ever stores ciphertext. **Both apps must follow this document exactly.**

Reference implementation: `crates/wirl-core/src/sync.rs` (keys, encryption, merge), `crates/wirl-core/src/relay.rs` (relay client, sync round), `relay/` (the Cloudflare Worker), `src-tauri/src/cloud_sync.rs` (desktop scheduling and pairing).

v1 (direct Wi-Fi between phone and computer) is retired. A v1 QR code (`"v":1`) must be rejected with "Update Wirl on your computer and scan its new code."

## Overview

- Devices that share one **group secret** form a sync group. A computer creates the secret the first time it shows its pairing code. Every phone that scans the code joins the same group, so they all share one dictionary.
- Each device derives three values from the secret: the **mailbox id**, the **mailbox token** and the **encryption key** (§3).
- The mailbox holds one encrypted value (the whole dictionary plus a device list) and a **version** number. Devices read it, merge their own changes, and write it back only if nobody wrote in between (compare-and-swap, §5).
- There is no server-side merge and no roles: computer and phones run the same sync round (§6).

## 1. Dictionary entry format

```json
{
  "id": "7d0f6b0e-3b1a-4c43-9a55-5d3c3c0a2f61",
  "term": "Sela",
  "soundsLike": ["salad", "sella"],
  "note": "my daughter's name",
  "updatedAt": 1767225600000,
  "deleted": false
}
```

| Field | Type | Meaning |
|---|---|---|
| `id` | string, UUID | Stable identity of the entry. Never reused. |
| `term` | string | The correct spelling. |
| `soundsLike` | array of strings | Known mishearings. May be empty. |
| `note` | string | Context note. May be empty. |
| `updatedAt` | integer | Milliseconds since the Unix epoch, UTC. Set on every create, edit and delete. |
| `deleted` | boolean | Tombstone flag. |

- **Delete** = set `deleted: true` and update `updatedAt`. The entry stays in the list (a *tombstone*) so the deletion syncs.
- **Purge** tombstones whose `updatedAt` is more than **90 days** old.
- When an entry is edited, `updatedAt` must strictly increase (`max(now, previous + 1)`).

## 2. Pairing

On the computer, *Dictionary › Pair a phone* shows a QR code. On the phone, *Computer sync › Pair with a computer* scans it. The QR content is this JSON (UTF-8):

```json
{
  "v": 2,
  "name": "DESKTOP-ABC",
  "relay": "https://wirl-relay.wirl.workers.dev",
  "secret": "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="
}
```

| Field | Meaning |
|---|---|
| `v` | Protocol version, `2`. |
| `name` | The computer's name, for display. |
| `relay` | Base URL of the mailbox relay (HTTPS). Phones use the URL from the code, so a different relay needs no phone update. |
| `secret` | Standard base64 (with padding) of the **32-byte group secret**. |

- The secret doesn't expire: the computer shows the same code until **Disconnect all phones** (§7) replaces it.
- Scanning a code for a computer the phone already has (same relay and mailbox id) replaces it; otherwise it's added. A phone may be in several groups (several computers).
- The phone keeps the secret in secure storage (Android: EncryptedSharedPreferences). The computer keeps it in Windows Credential Manager.

## 3. Keys

All three values use HKDF-SHA256 (RFC 5869) with **IKM = the 32 decoded secret bytes**, **no salt** (the RFC default: 32 zero bytes) and an output length of 32 bytes. Only the `info` label differs:

| Value | `info` (ASCII) | Encoding |
|---|---|---|
| Mailbox id | `wirl-mailbox-v2` | 64 lowercase hex characters |
| Mailbox token | `wirl-token-v2` | URL-safe base64 **without** padding (43 characters) |
| Encryption key | `wirl-sync-v2` | raw 32 bytes (AES-256 key) |

The relay stores **SHA-256(token)** as lowercase hex, never the token. Knowing the mailbox id or the token doesn't reveal the key.

## 4. Encryption

| Item | Value |
|---|---|
| Cipher | AES-256-GCM, 12-byte nonce, 16-byte tag |
| Key | the encryption key from §3 |
| Additional authenticated data | the mailbox id string (64 hex characters), ASCII bytes |
| `nonce` | standard base64 (with padding) of 12 fresh random bytes. **Never reuse** a nonce. |
| `ciphertext` | standard base64 of `encrypted bytes ‖ 16-byte tag` (the usual output of Java `Cipher`, Rust `aes-gcm`, Python `cryptography`) |

Plaintext (UTF-8 JSON):

```json
{
  "entries": [ /* the FULL dictionary, tombstones included */ ],
  "devices": [
    { "id": "0b7c6f7e-1d2a-4b5c-9d8e-7f6a5b4c3d2e", "name": "Pixel 9", "kind": "phone", "lastSync": 1767225600000 },
    { "id": "0c1a3a0e-8a57-4b0e-9d63-0a2b1d2e3f40", "name": "DESKTOP-ABC", "kind": "computer", "lastSync": 1767225500000 }
  ]
}
```

- `devices` lists everyone in the group. Each device's `id` is a UUID it creates once and keeps; `kind` is `"computer"` or `"phone"`; `lastSync` is when that device last uploaded (ms). Devices show the others' names and "Synced X ago" from this list.
- Unknown fields must be ignored and kept out of the way (don't fail on them).

A value that doesn't decrypt means the device has the wrong secret: treat it as "pair again" (§8).

### Test vector

Computed independently with Rust (`known_test_vector` in `sync.rs`) and Python's `cryptography` package. The Android app must pass the same vector.

| | |
|---|---|
| secret (base64) | `AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=` (bytes 0x00…0x1f) |
| mailbox id | `edaba0f8f5a6b41660b29dfc43e0c1f9c3067ab5d6cef6c3564a014f7752ee28` |
| token | `AJz6SLfpq2jeFQJFKtPqDMJjLGr4HmRTVsv4YRMp_Cw` |
| SHA-256(token) | `81d430e62ca3afa96592b7a7c50f6213ccbde0fd21a6cebfa6b0d5038facb16c` |
| encryption key (hex) | `beccb5b399d59b7385c4d5d5df2090be773deaf000144a2b2caa2a567232e841` |
| nonce (base64) | `AAECAwQFBgcICQoL` (bytes 0x00…0x0b) |
| plaintext | `{"entries":[],"devices":[]}` |
| ciphertext (base64) | `UcqqKN+j0S2YVtLI2StUerwpqF/KNKjI1Hzcac50dzPtEw5tYv5gpse67A==` |

## 5. Relay API

Base URL from the QR code, for example `https://wirl-relay.wirl.workers.dev`. Every mailbox request carries `Authorization: Bearer <token>`. Bodies are JSON.

### `GET {relay}/v2/m/{mailboxId}`

| Status | Body | Meaning |
|---|---|---|
| 200 | `{"version": 3, "nonce": "…", "ciphertext": "…"}` | The current value. |
| 404 | none | Empty mailbox (never written, or expired). |
| 401 | none | The mailbox belongs to a different token. |
| 410 | none | The mailbox was closed (§7). |

### `PUT {relay}/v2/m/{mailboxId}`

Body: `{"baseVersion": 3, "nonce": "…", "ciphertext": "…"}`. `baseVersion` is the version the device read (0 for an empty mailbox). `nonce` must be 16 base64 characters.

| Status | Body | Meaning |
|---|---|---|
| 200 | `{"version": 4}` | Stored. The new version is `baseVersion + 1`. |
| 409 | `{"version": 5, "nonce": "…", "ciphertext": "…"}` | Someone wrote first. This is the current value: merge with it and try again. |
| 400 | none | Malformed body. |
| 401 / 410 | none | As for GET. |
| 413 | none | Body over 1 MiB. |

An empty or expired mailbox accepts any `baseVersion` (the writer's merged data is the best there is). The first successful write sets the mailbox's token.

### `DELETE {relay}/v2/m/{mailboxId}`

Closes the mailbox: the dictionary is deleted and every later request with that token gets **410**. Answers 204 (also for an empty mailbox, which then becomes closed).

### Other rules

- `GET {relay}/` answers 200 with plain text (a health check).
- A mailbox nobody has touched for **180 days** is deleted (closed markers too).
- Requests for the same mailbox are handled one at a time, so compare-and-swap is reliable.

## 6. Sync round

Every device runs the same round:

1. **GET** the mailbox. 404 → no remote value, version 0.
2. **Decrypt** the value (if any).
3. **Merge** entries (§9). Drop tombstones older than 90 days. This merged list becomes the local dictionary.
4. **Decide whether to upload.** Upload if the merged entries differ from the mailbox's entries (compare sorted by `id`), or this device's record in `devices` is missing, has a different name or kind, or its `lastSync` is **15 minutes** old or more (a heartbeat, so others can show "Synced X ago").
5. **Upload**: `devices` = the mailbox's devices merged with this device's record (`lastSync` = now). Per `id`, the larger `lastSync` wins; drop devices not seen for **180 days**. PUT with `baseVersion` = the version read in step 1.
6. On **409**: use the returned value as the new remote and go back to step 2. Give up after 5 conflicts in one round (try again on the next round).

After a round, a device applies the result by merging it (§9) with its current dictionary, in case the user edited something while the round was running.

## 7. Disconnect all phones

On the computer, *Disconnect all phones* sends `DELETE` for the current mailbox, makes a **new** secret, and syncs its dictionary into the new mailbox. Phones on the old secret get **410** and show "pair again". The user re-scans the new code with the phones that should stay. (Removing a single phone would need accounts; this is the trade-off for having none.)

A phone's own *Unpair* only forgets the computer locally. It doesn't touch the mailbox.

## 8. Errors (what devices show)

| Situation | Message |
|---|---|
| 401, 410, or the value doesn't decrypt | "Pairing is no longer valid. Pair again." (phone: with a **Pair again** button) |
| No connection, timeout, DNS failure | "Can't reach the sync service. Will try again." |
| 5xx, or too many conflicts | "Sync didn't finish. Will try again." |

Failures are also logged in Diagnostics. Sync never blocks dictation or cleanup.

## 9. Merge rule

For each `id` present on either side:

1. The version with the **newer `updatedAt` wins**.
2. On a tie, the version with **`deleted: true` wins**.
3. On a full tie (same `updatedAt` and `deleted`), pick the version whose canonical JSON serialization sorts greater, so every device picks the same one.

The result is sorted by `id`.

## 10. When to sync

**Computer:** on launch, about 2 seconds after any dictionary change (a burst of edits is one sync), when the Dictionary page opens, on *Sync now*, and every **5 minutes** while Wirl runs.

**Phone:** when the Dictionary or Computer sync screen opens (in the foreground, with a "Syncing…" indicator), right after scanning a code, about 2 seconds after a dictionary change, when the network comes back, and every 30 minutes in the background (WorkManager).

Network timeouts: connect 10 s, whole request 20 s.
