# Wirl for Windows

Wirl lives in your system tray. Press a hotkey in any text box, talk, and Wirl types what you meant:

1. **Whisper** turns your speech into text **on your computer** (whisper.cpp, GPU via Vulkan when available).
2. **Claude** tidies it up: removes "um", follows "scratch that", fixes misheard words, and spells your names right.
3. Wirl **pastes** the result where your cursor is, then puts your clipboard back.

It's the desktop version of Wirl for Android. Both apps share a personal dictionary over Wi-Fi.

> **Your own API key.** Each person enters their own Anthropic API key in *Setup*. It is stored in Windows Credential Manager. No key is ever stored in this repository or used by GitHub Actions.

## Install (Windows 10/11, 64-bit)

1. Open the [latest release](https://github.com/tobijeh/Wirl-Desktop/releases/latest) and download `Wirl_x.y.z_x64-setup.exe`.
2. Run it. **The first time, Windows may say "Windows protected your PC".** Wirl isn't code-signed (that costs money every year), so Windows doesn't recognise it yet. Click **More info**, then **Run anyway**. You only need to do this once; updates install without this step.
3. Wirl opens a short **welcome tour** (about 2 minutes):
   - It explains the two things Wirl does: **talk, and it types for you**, and **select messy text, and it tidies it up**.
   - If something's missing, it asks for it: your **API key** from the [Anthropic Console](https://console.anthropic.com/settings/keys), and **microphone** access. The speech model (60 MB) downloads in the background.
   - You **try talking** in a practice box, then **tidy up** a messy paragraph, using the real shortcuts.
   - You can skip it, and take it again any time from *Setup › Take the tour*.
4. **Phone sync** (optional): *Dictionary › Pair with phone*. When Windows Firewall asks, allow Wirl on **Private networks** only.

Then press **Ctrl+Alt+Space** in any app. Hold it while you talk and let go to insert, or tap it once to start and again to stop. **Esc** cancels.
Select text and press **Ctrl+Alt+C** to clean it up; press it again straight away to undo.

## Updates and the signing key

Wirl updates itself from this repository's GitHub Releases. Every release is signed, and the app checks the signature before installing.

**This is set up.** The key has no password.

- The **public key** is in `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`). It isn't secret.
- The **private key** is the `TAURI_SIGNING_PRIVATE_KEY` secret (Settings › Secrets and variables › Actions) and, as a backup, on the owner's PC (`%USERPROFILE%\.tauri\wirl.key`). It is never committed: `.gitignore` blocks `*.key`.
- The Release workflow sets `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` to an empty string, because the key has no password.

**Releasing.** Every push to `main` builds the installer and publishes a release `vX.Y.Z` (the patch number goes up by one each time; change `version` in `src-tauri/tauri.conf.json` to start a new minor or major series). Each release has the installer `Wirl_X.Y.Z_x64-setup.exe`, its signature `.sig`, and `latest.json`, which installed copies check. You can also start it from **Actions › Release › Run workflow**. If the secret is missing, the release fails with a message saying so.

Installed copies check for updates on launch and once a day, download in the background, and show **Update ready** (green dot) in the tray. The update installs when you choose **Restart to update** or when you quit Wirl.

**If the private key is ever lost or leaked:** make a new pair (`npx.cmd @tauri-apps/cli@2 signer generate -w "$HOME\.tauri\wirl.key"`, press Enter twice for no password), replace the secret, and put the new public key in `tauri.conf.json`. Copies installed before that can't verify the new signature, so they need one manual reinstall from the next release.

## Develop

Requirements: Windows 10/11 x64, [Rust](https://rustup.rs) (MSVC), Node.js 22, CMake, Visual Studio Build Tools, and the [Vulkan SDK](https://vulkan.lunarg.com/) for the GPU sidecar.

```powershell
npm ci
cargo run -p wirl-icons -- all              # imports the icons (sheet or zip) if needed and builds app icons
./scripts/build-sidecars.ps1                # Whisper sidecars (add -CpuOnly to skip Vulkan)
npm run tauri dev                           # run
cargo test --workspace --exclude wirl-whisper
```

The icons are used exactly as provided. They come from `wirl-icons-sheet.png`, a 5×5 sheet: `cargo run -p wirl-icons -- slice` copies each 128×128 icon out of it pixel for pixel into `ui/assets/icons/` and writes `icons.json` (order as on the sheet, default `swirl`). These files are committed. A `wirl-icons.zip`, if added, is unzipped there instead (then deleted). `generate` only scales the default icon into the `.ico`/`.png` sizes Windows needs.

### Cleanup quality test (manual, uses your key)

```powershell
$env:ANTHROPIC_API_KEY = "sk-ant-..."     # only in this PowerShell window; never saved
cargo run -p wirl-core --example cleanup_quality
```

It runs 26 cases from `tests/quality/cases.json` (self-corrections, restarts, spoken commands, sound-alikes, dictionary terms, unchanged clean text, casual tone) through the exact same request the app sends. It is never run in GitHub Actions.

## Layout

| Path | What |
|---|---|
| `crates/wirl-core` | All platform-independent logic, unit tested: cleanup prompt and request, Claude client, dictionary and matching, sync crypto and merge, VAD chunking and `audio_ctx`, hotkeys, pill states, clipboard insert/undo logic, settings |
| `crates/wirl-whisper` | The Whisper sidecar process |
| `crates/wirl-icons` | Icon import and `.ico` generation |
| `src-tauri` | The Tauri app: tray, hotkeys, audio, pill window, Windows integration, sync server, updater |
| `ui` | Settings window and pill (plain HTML/CSS/JS, Manrope font bundled) |
| `docs/sync-protocol.md` | Phone sync protocol (shared with Android) |

See [CLAUDE.md](CLAUDE.md) for the design and rules, and [FEATURES.md](FEATURES.md) for feature status on Android and Desktop.

## Privacy

- Audio never leaves your computer.
- Text is sent to Anthropic only for cleanup, with your own key. Turn AI cleanup off to keep everything local.
- Dictionary sync is end-to-end encrypted (AES-256-GCM) and stays on your local network.
