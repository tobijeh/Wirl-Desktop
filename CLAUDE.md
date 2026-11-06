# CLAUDE.md: Wirl for Windows

Guidance for anyone (human or AI) changing this repo. Read this first.

## What Wirl is

A system-tray dictation app: hotkey → record → Whisper (local) → Claude cleanup → paste at the cursor → restore clipboard. Desktop twin of the Wirl Android app; the two share a dictionary over Wi-Fi.

## Hard rules

1. **No API keys in the repo or in GitHub Actions.** Each user enters their own Anthropic key in Setup; it lives in Windows Credential Manager (`keyring`). The quality-test script reads `ANTHROPIC_API_KEY` from the environment at run time and never saves it. Never add a workflow step that calls the Claude API.
2. **The core cleanup prompt is fixed.** `wirl_core::prompt::CORE_PROMPT` must match the product spec character for character and is not user-editable. Only style notes and matching dictionary entries are appended (in that order, under the exact headers in `prompt.rs`).
3. **Cleanup request:** model `claude-haiku-5-5`, `thinking: {type: "disabled"}`, `output_config.effort: "low"`, no sampling parameters, streamed. One shared function (`ClaudeClient::cleanup`) for dictation, clean-selection and the quality script.
4. **Icons are used exactly as provided.** The 25 PNGs from `wirl-icons.zip` (in `ui/assets/icons/`, order and default from `icons.json`) are never redrawn, restyled or substituted. Only scaling for `.ico`/tray sizes and the tray-state overlays (red ring, dimming, greyscale, amber badge, green dot) are allowed.
5. **Sync protocol is shared with Android.** Any change must be made in `docs/sync-protocol.md` first and stay wire-compatible. The `known_test_vector` test pins the crypto.
6. **Whisper runs in a sidecar process** (`crates/wirl-whisper`). Never link whisper.cpp into the app process. If the sidecar dies, the app logs why, fails pending requests, restarts it and reloads the model.
7. **Nothing heavy on the UI thread.** Capture has its own thread, Whisper its own process, clipboard/SendInput work runs in `spawn_blocking`. Every stage has a timeout (`wirl_core::pipeline`); the watchdog resets anything busy over 30 s.
8. **Plain language, autosave.** UI text is short and non-technical. No Save buttons; every change applies immediately.
9. **Portable core.** OS-specific code stays in `src-tauri/src/platform/` (Windows implementation + stubs). Logic that can be tested without an OS goes in `wirl-core` with unit tests.

## Architecture

```
crates/wirl-core      pure logic (unit tested, builds anywhere)
  prompt.rs           core prompt, system prompt builder, request body
  claude.rs           keep-alive HTTP client, SSE parsing, warm(), error → plain message
  dictionary.rs       entries, search, phonetic matching, import/export, merge, tombstones
  sync.rs             pairing QR, HKDF + AES-GCM, request handling, private-peer check
  audio.rs            resampler → 16 kHz mono, VAD chunker (5–10 s chunks), audio_ctx
  filter.rs           strip [sound tags] and lone hallucinations
  hotkey.rs           parse/format, conflicts, shortcut recorder, hold-to-talk state machine
  pill.rs             pill states, fade timings, placement
  insert.rs           clipboard insert + restore + fallback; clean-selection undo tracker
  pipeline.rs         discard rules, cleanup fallback, timeouts
  settings.rs, models.rs (catalog + resumable verified download), ipc.rs, icons.rs, diagnostics.rs, watchdog.rs
crates/wirl-whisper   sidecar: JSON lines over stdin/stdout, whisper-rs
crates/wirl-icons     unzip wirl-icons.zip → ui/assets/icons, generate src-tauri/icons
src-tauri/src
  lib.rs              plugins, setup, run loop (closing the window keeps Wirl in the tray)
  app.rs              shared state (App), session counter, attention items
  dictation.rs        dictation + clean-selection flows, watchdog task
  sidecar.rs          spawn/restart/GPU→CPU fallback, request routing, timeouts
  audio.rs            cpal capture thread, chime
  hotkeys.rs          global shortcuts (Cancel only while recording), conflicts
  pill.rs, tray.rs    indicator window and tray icon/menu
  sync_server.rs      axum server, mDNS, pairing
  updater.rs          check on launch + daily, download, install on restart/quit
  commands.rs         settings-window API
  platform/windows.rs clipboard (all formats), SendInput, UIPI check, monitors, media pause, mic privacy
ui/                   settings.html/js, pill.html/js, pill-view.js (shared renderer), theme.css
```

### Dictation flow

1. Dictate pressed → `DictateTrigger` decides start/stop (tap toggles; hold ≥350 ms records until release when Hold to talk is on).
2. Start: pause playing media, open mic, register Cancel, warm the Claude connection, make sure the model is loaded, show the Recording pill.
3. While recording, the chunker cuts at natural pauses after ≥5 s (forced by 10 s); each speech chunk is transcribed in the background with `audio_ctx` sized to its length and dictionary terms as the initial prompt.
4. Stop: the final chunk is sent immediately. Recordings under 1 s or with no speech are discarded silently.
5. Join → filter. Nothing left = no speech (silent discard).
6. Cleanup with matching dictionary entries. Failure inserts the raw transcript and shows the reason.
7. Insert via clipboard + Ctrl+V, restore the clipboard if nobody changed it. Blocked (elevated window etc.) → leave text on clipboard, "Copied, press Ctrl+V".

### Clean up selection

Copy selection → `UndoTracker::on_hotkey`. Same text as the last cleanup → paste the original (undo). No selection but armed → re-select the pasted text with Shift+Left, compare, undo if unchanged; otherwise collapse the selection and reset. Different text → clean it.

## Pill states

Hidden when idle. Recording (red ring, wave, "<Cancel> to cancel"), Processing ("Cleaning up"), Inserted (1 s), Cleaned + "<Clean hotkey> to undo" (3 s), Undone, Copied (amber), Error (amber, short message). The "status messages" toggle hides only the success confirmations.

## Tray states

Ready, Recording (red ring), Processing (dimmed, pulsing), Off (greyscale), Needs attention (amber badge), Update ready (green dot). Priority: Recording > Processing > Off > Needs attention > Update ready > Ready.

## Commands

```bash
cargo test --workspace --exclude wirl-whisper     # unit tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
node scripts/check-ui.mjs                          # UI syntax
cargo run -p wirl-icons -- all                     # icons (needs wirl-icons.zip or ui/assets/icons)
./scripts/build-sidecars.ps1                       # Windows sidecars
npm run tauri dev
cargo run -p wirl-core --example cleanup_quality   # manual, needs ANTHROPIC_API_KEY in your shell
```

Cross-checking Windows code from Linux: `cargo clippy -p wirl --target x86_64-pc-windows-gnu` (with mingw-w64 installed and placeholder sidecars in `src-tauri/binaries`).

## CI / releases

- `.github/workflows/ci.yml` (pull requests): fmt, UI syntax, clippy, unit tests, then a full Windows build (sidecars + NSIS installer) uploaded as an artifact.
- `.github/workflows/release.yml` (push to `main`): next patch version from the latest `vX.Y.*` tag, signed build, GitHub Release with `latest.json`. Needs the `TAURI_SIGNING_PRIVATE_KEY` secret and `WIRL_UPDATER_PUBKEY` variable (README). Skips with a warning without them.
- No paid code signing; README documents the "Windows protected your PC" step.

## Style

- Theme tokens live in `ui/theme.css` (background #111116, sidebar #16151D, cards #1C1B24 with #2C2A38 border, accent #8C7CF7, success #7DDCAE, warning #F2B66B, red #E5484D, text #F2F1F7 / #A9A7B8, Manrope bundled).
- Cards 16–20 px radius. Amber for anything that needs the user; green for done.
- When you add a feature, update FEATURES.md.
