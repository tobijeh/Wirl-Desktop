# CLAUDE.md: Wirl for Windows

Guidance for anyone (human or AI) changing this repo. Read this first.

## What Wirl is

A system-tray dictation app: hotkey → record → Whisper or SenseVoice (local) → Claude cleanup → paste at the cursor → restore clipboard. Desktop twin of the Wirl Android app; the two share a dictionary through an end-to-end encrypted mailbox (no accounts).

## Hard rules

1. **No API keys in the repo or in GitHub Actions.** Each user enters their own Anthropic key in Setup; it lives in Windows Credential Manager (`keyring`). The quality-test script reads `ANTHROPIC_API_KEY` from the environment at run time and never saves it. Never add a workflow step that calls the Claude API.
2. **The core cleanup prompt is fixed.** `wirl_core::prompt::CORE_PROMPT` must match the product spec character for character and is not user-editable. Only style notes and matching dictionary entries are appended (in that order, under the exact headers in `prompt.rs`). Exception: when the Cantonese or Mandarin model produced the transcript, `CANTONESE_BLOCK` or `MANDARIN_BLOCK` goes right after the core prompt (see Speech models below).
3. **Cleanup request:** model `claude-haiku-5-5`, `thinking: {type: "disabled"}`, `output_config.effort: "low"`, no sampling parameters, streamed. One shared function (`ClaudeClient::cleanup`) for dictation, clean-selection and the quality script.
4. **Icons are used exactly as provided.** The 25 PNGs cut from `wirl-icons-sheet.png` (or a `wirl-icons.zip`) (in `ui/assets/icons/`, order and default from `icons.json`) are never redrawn, restyled or substituted. Only scaling for `.ico`/tray sizes and the tray-state overlays (red ring on the tray icon only, dimming, greyscale, amber badge, green dot) are allowed.
5. **Sync protocol is shared with Android.** Any change must be made in `docs/sync-protocol.md` first and stay wire-compatible. The `known_test_vector` test pins the crypto.
6. **Speech engines run in sidecar processes** (`crates/wirl-whisper`, `crates/wirl-sensevoice`). Never link whisper.cpp or sherpa-onnx into the app process. If the sidecar dies or times out, the app logs why, fails pending requests, restarts it and reloads the model.
7. **Nothing heavy on the UI thread.** Capture has its own thread, Whisper its own process, clipboard/SendInput work runs in `spawn_blocking`. Every stage has a timeout (`wirl_core::pipeline`); the watchdog resets anything busy over 30 s.
8. **Plain language, autosave.** UI text is short and non-technical. No Save buttons; every change applies immediately.
9. **Portable core.** OS-specific code stays in `src-tauri/src/platform/` (Windows implementation + stubs). Logic that can be tested without an OS goes in `wirl-core` with unit tests.

## Architecture

```
crates/wirl-core      pure logic (unit tested, builds anywhere)
  prompt.rs           core prompt, system prompt builder, request body
  claude.rs           keep-alive HTTP client, SSE parsing, warm(), error → plain message
  dictionary.rs       entries, search, phonetic matching, import/export, merge, tombstones
  sync.rs             pairing QR v2, key derivation (mailbox id, token, key), AES-GCM, payload + device merge
  relay.rs            mailbox relay client (GET/PUT/DELETE) and the sync round with compare-and-swap retries
  audio.rs            resampler → 16 kHz mono, VAD chunker (5–10 s chunks), audio_ctx
  filter.rs           strip [sound tags] and lone hallucinations
  hotkey.rs           parse/format, conflicts, shortcut recorder, hold-to-talk state machine
  pill.rs             pill states, fade timings, placement
  insert.rs           clipboard insert + restore + fallback; clean-selection undo tracker
  pipeline.rs         discard rules, cleanup fallback, timeouts
  onboarding.rs       usage counts, one-time clean-up tip, shortcut card rule
  settings.rs, models.rs (catalog + resumable verified download), ipc.rs, icons.rs, diagnostics.rs, watchdog.rs
crates/wirl-whisper   sidecar: JSON lines over stdin/stdout, whisper-rs
crates/wirl-sensevoice  sidecar for the Cantonese and Mandarin models: same protocol, sherpa-onnx (static, no DLLs)
relay/                Cloudflare Worker: one Durable Object per mailbox (rules in src/mailbox.js, node tests)
crates/wirl-icons     slice wirl-icons-sheet.png / unzip wirl-icons.zip → ui/assets/icons, generate src-tauri/icons
src-tauri/src
  lib.rs              plugins, setup, run loop (closing the window keeps Wirl in the tray)
  app.rs              shared state (App), session counter, attention items
  dictation.rs        dictation + clean-selection flows, watchdog task
  sidecar.rs          spawn/restart/GPU→CPU fallback, Whisper↔SenseVoice switch, request routing, timeouts
  audio.rs            cpal capture thread, chime
  hotkeys.rs          global shortcuts (Cancel only while recording; Switch speech model), conflicts
  pill.rs, tray.rs    indicator window and tray icon/menu
  cloud_sync.rs       sync loop (launch, after changes, every 5 min), pairing code, disconnect all phones
  updater.rs          check on launch + daily, download, install on restart/quit
  commands.rs         settings-window API
  platform/windows.rs clipboard (all formats), SendInput, UIPI check, monitors, media pause, mic privacy
ui/                   settings.html/js, pill.html/js, pill-view.js (shared renderer), tour-logic.js (welcome tour), theme.css
```

### Dictation flow

1. Dictate pressed → `DictateTrigger` decides start/stop (tap toggles; hold ≥350 ms records until release when Hold to talk is on).
2. Start: pause playing media, open mic, register Cancel, warm the Claude connection, make sure the model is loaded, show the Recording pill.
3. While recording, the chunker cuts at natural pauses after ≥5 s (forced by 10 s); each speech chunk is transcribed in the background with `audio_ctx` sized to its length and dictionary terms as the initial prompt.
4. Stop: the final chunk is sent immediately. Recordings under 1 s or with no speech are discarded silently. Pauses and the end of speech are found relative to the room's background (measured in the first 240 ms), not only the fixed speech threshold, so soft speakers, quiet microphones and trailing-off last words aren't trimmed as silence (`Chunker::is_voice`; tests in `audio.rs`). The room is only measured from a **steady** sound (loudest frame ≤ 2.5× the quietest): someone already talking when the key goes down isn't taken as the room. Quiet voice counts as speech after 90 ms in a row or 240 ms in total (soft syllables come in bursts). A piece with no clear speech but some **faint** sound (≥ 2 frames above 1.8× the room level) in a recording that has speech (`Chunk::weak_speech`, typically the soft end of a sentence after a pause cut) is still transcribed, and its text kept only if it's real words (`pipeline::weak_chunk_text`: not a lone hallucination or sound tag, ≥ 3 letters). Diagnostics note every skipped, faint or failed piece.
5. Join → filter. Nothing left = no speech (silent discard). **Fewer than 3 letters** (`pipeline::MIN_LETTERS`, Chinese characters count) is noise: Claude isn't asked, nothing is inserted or shown, and Diagnostics notes it (same rule as Android's `SpeechGate`).
6. Cleanup with matching dictionary entries. Failure inserts the raw transcript and shows the reason. A reply that's empty, has no letters or digits (a lone "."), or is a remark about the recording ("No speech detected.", `pipeline::is_recording_remark`, Android's list) inserts nothing and shows nothing.
7. Insert via clipboard + Ctrl+V. With **Keep what you dictate on the clipboard** (`Settings.keep_on_clipboard`, default on, Dictation page) the text stays on the clipboard (`AfterPaste::Keep`) and in clipboard history (Win+V; never cloud clipboard, `clipboard_set_text_kept`), so a paste that didn't land can be repeated with Ctrl+V. Off: the previous clipboard is restored after 400 ms if nobody changed it, and the text is kept out of clipboard history. Blocked (elevated window etc.) → leave text on clipboard, "Copied, press Ctrl+V". **Making the paste land** (`insert_text_then`): after setting the clipboard Wirl waits 60 ms, and up to 300 ms more while another program holds the clipboard open (`settle`, `wait_clipboard_free`; Diagnostics names that program). Ctrl+V is typed like a person (Ctrl down, 15 ms, V with scan codes, 15 ms, Ctrl up: `send_ctrl_key`), because some apps miss a one-burst shortcut. The focused text box is read through UI Automation before and after (`probe_text_box` / `text_box_changed`, 400 ms UIA timeouts, password boxes and text over 200,000 characters skipped): if it hasn't changed after 1 s, Ctrl+V is sent once more; still unchanged → the text stays on the clipboard and the pill says "Copied, press Ctrl+V" (`PasteError::NotLanded`). Apps whose text can't be read are pasted into unchecked, as before. Diagnostics note the check result (`check_note`).

### Clean up selection

Copy selection → `UndoTracker::on_hotkey`. Same text as the last cleanup → paste the original (undo). No selection but armed → re-select the pasted text with Shift+Left, compare, undo if unchanged; otherwise collapse the selection and reset. Different text → clean it.

## Phone sync (desktop side)

- Protocol v2 in `docs/sync-protocol.md`: a mailbox relay on Cloudflare (`relay/`, deployed by `.github/workflows/relay.yml`), no accounts. The relay stores only ciphertext and SHA-256 of the token. Never send anything unencrypted to it, and never add server-side logic that needs to read the dictionary.
- The group secret lives in Windows Credential Manager (`storage::get_sync_secret`). It's created the first time the pairing code is shown and stays the same until "Disconnect all phones" (DELETE the mailbox → phones get 410, then a new secret).
- `cloud_sync` runs one round at a time (`RUNNING` lock): on launch, 2 s after a dictionary change (`cloud_sync::request()` from every dictionary command), when the Dictionary page opens, on "Sync now", and every 5 minutes. The result is merged with the current dictionary (never overwrites edits made during the round). New phones in the device list fire `wirl://paired`.
- Dictionary fields (UI labels; the JSON names `term`, `soundsLike`, `note` don't change): **Correct spelling**, **Wirl types it as** (the wrong text Wirl actually typed; that's what matching and the prompt use, sent as "misheard as"), **Only when** (optional; only for wrong words that are also real words). Keep the labels in step with the Android app.
- UI: the Phone sync card sits at the **top** of the Dictionary page (one compact row: phones and "synced X ago", Sync now, Pair). The pairing code and "Disconnect all phones" open inside it. Don't move it below the term list: long dictionaries hid it.
- Relay URL: `wirl_core::sync::DEFAULT_RELAY` (`https://wirl-relay.wirl.workers.dev`), overridable at build time with `WIRL_RELAY_URL`. Phones take it from the QR code.
- Manual end-to-end check: run the relay locally (`cd relay && npx wrangler@4 dev --local --port 8799`) and `WIRL_RELAY_URL=http://127.0.0.1:8799 cargo run -p wirl-core --example relay_smoke`.

## Speech models (one per language)

- The Dictation page has one **Speech model** list, one row per model, each naming its language (same names and wording as Android): **English · Fast** (Whisper base.en), **English · Accurate** (Whisper small.en), **Cantonese (粵語)**, **Mandarin (普通话)**. There is **no separate language setting**: the model decides what the engine is told (`en`, `yue`, `zh`: `SpeechLanguage::code`) and the cleanup mode (`SpeechLanguage::prompt_mode`), so they can't disagree. The model name (`ModelInfo::name`) is used everywhere it's shown.
- **Multilingual was removed.** `Settings::from_json` migrates old settings (`SETTINGS_VERSION` 2): `multilingual` (or any unknown model) becomes `fast`, the old `language` key is dropped, and the app deletes `ggml-base.bin` at start-up (`App::remove_retired_models`).
- Rows show size, progress, Cancel (keeps the `.part` file; the row offers Resume), Download or Delete (asks first, in the row). The model in use, when not downloaded, gets an amber note. Downloads are tracked **per file**. Clicking a row never starts a download; only the Download button does. "Use now" makes a downloaded model the one in use (`use_model`).
- **Quick switch** (same rules as Android #62, `models::cycle`, port of `ModelCycle`): each row has a tick box (`Settings.quick_switch`, always counting the model in use). Only a downloaded model can be ticked; one always stays ticked; unticking the one in use moves to the next ticked model (downloaded first). A finished download ticks its model and becomes the one in use if that one isn't downloaded; Delete unticks. The **Switch speech model** hotkey (default Ctrl+Alt+M, registered with Dictate and Clean up) moves to the next ticked, downloaded model in list order, wrapping round (Android's swipe up/down; the desktop goes forward only). Idle: the pill shows the model's tag (`ModelInfo::tag`: EN, EN+, 粵, 普) and name for 1.4 s (`ModelSwitched`), or "Tick more models to switch" when there's nothing to switch to. Recording: what was said is dropped as with Cancel, recording starts again in the new model (the Dictate key's hold state is kept), and the recording pill shows the tag in place of the icon, with the model name, for 1.2 s (`Recording.tag`, `TAG_FLASH_MS`), then the usual icon. Ignored while processing or cleaning up a selection.
- **Cantonese and Mandarin** run SenseVoice (`sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09`, int8, official k2-fsa `asr-models` release) through sherpa-onnx in `wirl-sensevoice`, with the **same files** (one download; rows say "one download for Cantonese and Mandarin"; deleting either removes both, with a warning). Details: `docs/cantonese.md`, `docs/mandarin.md`.
- The SenseVoice archive (165,783,878 bytes, SHA-256 checked) is downloaded in the app, never bundled; only `model.int8.onnx` (237,115,547 bytes) and `tokens.txt` (315,894 bytes) are unpacked, sizes checked, then the archive is deleted.
- sherpa-onnx is the official `sherpa-onnx` crate, pinned exactly (`=2.0.1`, same as Android), linked statically. Its Windows libraries are built with /MT, so the sidecar is built with `-C target-feature=+crt-static` (`build-sidecars.ps1`, its own target folder). It's excluded from the lint job's clippy/tests and checked in the build job instead (clippy, start-up smoke test, real recognition on a test clip).
- Language is fixed per model (`yue` / `zh`, never `auto`) and sent with `Load`, so switching Cantonese ↔ Mandarin reloads with the other language. `use_itn` is on. All-capital English from SenseVoice is lower-cased; chunks are joined without a space between two Chinese pieces (a space stays next to English). Diagnostics name the stages "SenseVoice chunk N" / "SenseVoice after stop".
- Cleanup: `PromptMode::Cantonese` / `PromptMode::Mandarin` add `CANTONESE_BLOCK` / `MANDARIN_BLOCK` after the core prompt, before style notes, for dictations from those models only (clean-selection is unchanged). **Both blocks must stay identical in the Android (Wirlwind) and desktop repos**; `docs/cantonese.md` and `docs/mandarin.md` hold the shared copies (the Android repo has the same two files) and unit tests check the code matches them.
- Dictionary entries written in Chinese characters are always sent in Cantonese and Mandarin mode (`Dictionary::matching_chinese`; the model writes simplified, entries are often traditional).

## Diagnostics (About › Developer options)

- Hidden until the version in About is clicked 7 times (`reveal_developer_options`), like Android. Shows the speech engine status (running, GPU/CPU, restarts, last error), the data folder, and the last 20 attempts (`wirl_core::diagnostics`, in memory only, never saved or sent).
- Each entry: per-stage timings (engine chunks, "… after stop", Claude first byte and total, Insert, and **Stop to insert** for dictations), model, thinking, **Claude connection** ("warm (already open)" / "new": `CleanupResult::warm_connection`, a request within 110 s of the last answer; the pool keeps connections 120 s), insert method, error, and the text at each step like Android: **Heard** (the filtered transcript, or "Selected" for clean-up), **Claude returned**, **Inserted**.
- **Paste notes** on each dictation and clean-up (`add_paste_notes`): characters pasted (and how many are letters or digits), the program in front when pasting and whether it's the window the dictation started in (`platform::foreground_app`, program file name only, never the window title), Ctrl/Alt/Shift/Win still held after the 1.5 s wait (`held_modifiers`), and other dictation apps running (`running_dictation_apps`: Wispr Flow, Superwhisper, Dragon, Windows Voice Access, Talon). Discards note why nothing was inserted. These are for "Inserted but nothing appeared" reports: "Inserted" only means Ctrl+V was sent.
- **Copy report** (`copy_diagnostics`) puts a plain-text report on the clipboard: version, system, model and quick switch, engine status, cleanup on/off, API key saved/missing (never the key), hotkeys, attention items, phone sync, log folder, then every entry (`diagnostics::report`, text capped at 2,000 characters each). The page says it includes recent dictated text. **Open log folder** (`open_log_folder`, `platform::open_folder`) opens the folder with `wirl.log`.

## Welcome tour (onboarding)

- First launch opens the settings window on `#tour` (full window, no sidebar): Welcome → Get ready (only if the microphone or API key is missing; the speech model starts downloading in the background) → Try talking → Tidy up some text → You're all set. `first_run_done` is set only when the tour is finished or skipped, so closing the window halfway brings it back next launch. Anyone can replay it from Setup ("Take the tour") or the Home shortcut card.
- The practice boxes live in the settings window and use the real hotkeys: dictation and clean-up paste into the focused box exactly as in any other app. Hotkeys shown are the user's own.
- Content and rules (practice script, messy paragraph, which steps show, the "dropped the um" notes, clean/undo detection) are in `ui/tour-logic.js`, tested by `scripts/test-ui.mjs`. Notes only describe what actually happened.
- After the tour: `Settings.usage` (`wirl_core::onboarding`) counts successful dictations and clean-ups. Home shows "Your two shortcuts" until each has been used 3 times. After the 2nd dictation, if clean-up has never been used, the pill shows "Inserted" with one tip ("Tip: select text, then <clean hotkey>") once, ever; it waits while status messages are off. Usage survives "Reset to defaults".
- Tone: warm and brief, plain words.

## Pill states

Hidden when idle. Recording (the plain icon, the purple Wirl logo by default, and the live wave, nothing else, like the bubble on Android: no cancel text, no ring; right after a quick switch the model tag replaces the icon, with the model name, for 1.2 s, then the icon is back; a missing icon falls back to the Wirl logo), Model switched (tag + name, 1.4 s; always shown), Processing ("Cleaning up"), Inserted (1 s), Inserted + one-time clean-up tip (6 s), Cleaned + "<Clean hotkey> to undo" (3 s), Undone, Copied (amber), Error (amber, short message). The "status messages" toggle hides only the success confirmations.

## Tray states

Ready, Recording (red ring), Processing (dimmed, pulsing), Off (greyscale), Needs attention (amber badge), Update ready (green dot). Priority: Recording > Processing > Off > Needs attention > Update ready > Ready.

## Commands

```bash
cargo test --workspace --exclude wirl-whisper --exclude wirl-sensevoice   # unit tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
node scripts/check-ui.mjs                          # UI syntax
node scripts/test-ui.mjs                           # UI logic (welcome tour)
node --test relay/test/mailbox.test.mjs            # sync relay rules
cargo run -p wirl-icons -- all                     # icons (ui/assets/icons, else wirl-icons-sheet.png or wirl-icons.zip)
./scripts/build-sidecars.ps1                       # Windows sidecars (Whisper GPU + CPU, SenseVoice)
npm run tauri dev
cargo run -p wirl-core --example cleanup_quality   # manual, needs ANTHROPIC_API_KEY in your shell
```

Cross-checking Windows code from Linux: `cargo clippy -p wirl --target x86_64-pc-windows-gnu` (with mingw-w64 installed and placeholder sidecars in `src-tauri/binaries`).

## CI / releases

- `.github/workflows/ci.yml` (pull requests): fmt, UI syntax, clippy, unit tests, then a full Windows build (sidecars + NSIS installer) uploaded as an artifact.
- `.github/workflows/release.yml` (push to `main`): next patch version from the latest `vX.Y.*` tag, signed build, GitHub Release with the installer, its `.sig` and `latest.json`. Needs the `TAURI_SIGNING_PRIVATE_KEY` secret; fails with a clear message without it.
- `.github/workflows/relay.yml` (push to `main` touching `relay/`): tests and deploys the sync relay to Cloudflare. Needs the `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` secrets; skips the deploy with a notice without them.
- No paid code signing; README documents the "Windows protected your PC" step.
- **Updater signing key.** The public key lives in `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`, with `bundle.createUpdaterArtifacts: true`). The private key lives only in the `TAURI_SIGNING_PRIVATE_KEY` GitHub secret and on Toby's PC. It has no password, so `release.yml` sets `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` to `""`. Never commit a private key (`.gitignore` blocks `*.key`), and never change the public key unless the private key is replaced: installed copies only accept updates signed with the matching key. Pull request builds turn updater artifacts off, because they don't get the key.
- **Whisper bindings** are generated by bindgen on the Windows runner (`LIBCLANG_PATH`, LLVM is preinstalled). Don't set `WHISPER_DONT_GENERATE_BINDINGS`: the bundled bindings are for Linux and fail to compile for MSVC. The sidecars build with `CMAKE_GENERATOR=Ninja` inside the MSVC environment (`ilammy/msvc-dev-cmd`), because under the Visual Studio generator whisper.cpp's `vulkan-shaders-gen` sub-build can't find the compiler. They build in a short folder (`WIRL_SIDECAR_TARGET=C:\s`) because the Vulkan build's nested paths pass the Windows path limit otherwise.

## Style

- Theme tokens live in `ui/theme.css` (background #111116, sidebar #16151D, cards #1C1B24 with #2C2A38 border, accent #8C7CF7, success #7DDCAE, warning #F2B66B, red #E5484D, text #F2F1F7 / #A9A7B8, Manrope bundled).
- Cards 16–20 px radius. Amber for anything that needs the user; green for done.
- When you add a feature, update FEATURES.md.
