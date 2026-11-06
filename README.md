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
3. Wirl opens *Setup*:
   - **API key**: paste your key from the [Anthropic Console](https://console.anthropic.com/settings/keys).
   - **Speech model**: click *Fix* to download the Fast model (60 MB) once.
   - **Microphone**: if this is amber, click *Fix* and allow desktop apps to use the microphone.
   - **Phone pairing** (optional): when Windows Firewall asks, allow Wirl on **Private networks** only.
4. Click into the test box, press **Test dictation**, say something, press **Stop**.

Then press **Ctrl+Alt+Space** in any app. Hold it while you talk and let go to insert, or tap it once to start and again to stop. **Esc** cancels.
Select text and press **Ctrl+Alt+C** to clean it up; press it again straight away to undo.

## Updates and the signing key (one-time setup for the repo owner)

Wirl updates itself from this repository's GitHub Releases. Every release is signed, and the app checks the signature before installing. GitHub Actions needs the **private** signing key to sign releases. It's the only secret this repo needs.

**1. Create the key pair** (on your own computer, once). You need [Node.js](https://nodejs.org) installed. In PowerShell:

```powershell
npx @tauri-apps/cli@2 signer generate -w "$HOME\.tauri\wirl.key"
```

When it asks for a password, you can press Enter twice for no password (simplest). It creates two files:

- `%USERPROFILE%\.tauri\wirl.key`: the **private key**. Keep it secret and back it up (for example in your password manager). If you lose it, existing installs can't receive updates.
- `%USERPROFILE%\.tauri\wirl.key.pub`: the **public key**. Not secret.

**2. Add the private key as a secret.** On GitHub: this repo › **Settings** › **Secrets and variables** › **Actions** › **Secrets** tab › **New repository secret**.

- Name: `TAURI_SIGNING_PRIVATE_KEY`
- Secret: the entire contents of `wirl.key`. To copy it: `Get-Content "$HOME\.tauri\wirl.key" | Set-Clipboard`, then paste.
- Click **Add secret**.
- Only if you chose a password in step 1: add a second secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` with that password.

**3. Add the public key as a variable.** Same page, **Variables** tab › **New repository variable**.

- Name: `WIRL_UPDATER_PUBKEY`
- Value: the entire contents of `wirl.key.pub` (`Get-Content "$HOME\.tauri\wirl.key.pub" | Set-Clipboard`).

**4. Release.** Every push to `main` now builds the installer and publishes a release `vX.Y.Z` with `latest.json` for the updater (the patch number goes up by one each time; change `version` in `src-tauri/tauri.conf.json` to start a new minor/major series). Until the key is added, the Release workflow skips publishing and shows a warning instead of failing. You can also start it from **Actions › Release › Run workflow**.

Installed copies check for updates on launch and once a day, download in the background, and show **Update ready** (green dot) in the tray. The update installs when you choose **Restart to update** or when you quit Wirl.

## Develop

Requirements: Windows 10/11 x64, [Rust](https://rustup.rs) (MSVC), Node.js 22, CMake, Visual Studio Build Tools, and the [Vulkan SDK](https://vulkan.lunarg.com/) for the GPU sidecar.

```powershell
npm ci
cargo run -p wirl-icons -- all              # imports wirl-icons.zip (if present) and builds app icons
./scripts/build-sidecars.ps1                # Whisper sidecars (add -CpuOnly to skip Vulkan)
npm run tauri dev                           # run
cargo test --workspace --exclude wirl-whisper
```

The icons in `wirl-icons.zip` are used exactly as provided: `wirl-icons` unzips them into `ui/assets/icons/` (then deletes the zip) and only scales the default icon into the `.ico`/`.png` sizes Windows needs.

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
