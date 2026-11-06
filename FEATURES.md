# Features

Status per platform.

**Desktop:** ✅ done and covered by unit tests · 🟢 done, builds for Windows, needs a check on a real Windows PC · ⬜ not planned
**Android:** 🔵 exists (stated in the desktop spec) · ❔ please confirm (the Android column was filled in from the desktop spec, not from the Android code)

| Area | Feature | Desktop | Android |
|---|---|---|---|
| **Dictation** | Local Whisper transcription (whisper.cpp) | 🟢 sidecar process, verified on Linux CPU | ❔ |
| | Vulkan GPU with automatic CPU fallback | 🟢 | ⬜ |
| | Sidecar crash/OOM isolation, auto-restart, reason logged | 🟢 | ⬜ |
| | Background chunking at natural pauses (5–10 s) | ✅ | ❔ |
| | Final chunk starts the instant recording stops | 🟢 | ❔ |
| | `audio_ctx` sized per chunk (min 384) | ✅ | ❔ |
| | Dictionary terms as Whisper initial prompt | ✅ | ❔ |
| | Sound-tag and lone-hallucination filtering | ✅ | ❔ |
| | Silent discard (< 1 s or no speech) | ✅ | ❔ |
| | Insert at cursor via clipboard paste, clipboard restored | ✅ logic / 🟢 Windows | ❔ |
| | "Copied, press Ctrl+V" fallback when paste is blocked | ✅ logic / 🟢 Windows | ❔ |
| | Pause media during dictation (Windows media controls) | 🟢 | ❔ |
| | Start/stop chime | 🟢 | ❔ |
| **Models** | Fast (base.en q5_1), Accurate (small.en), Multilingual (base) | ✅ | ❔ |
| | On-demand download from Hugging Face with progress, resume, SHA-256 check | ✅ | ❔ |
| | Active model kept loaded | 🟢 | ❔ |
| | Language picker | 🟢 | ❔ |
| **AI cleanup** | Claude Haiku 5.5, thinking off, exact built-in prompt | ✅ | 🔵 same app family |
| | Personal style notes | ✅ | ❔ |
| | Only plausibly matching dictionary entries sent | ✅ | ❔ |
| | Keep-alive client, connection warmed at record start | 🟢 | ❔ |
| | Raw-text fallback with plain reason | ✅ | ❔ |
| | Line breaks and lists preserved | ✅ | ❔ |
| | Own API key, stored in the OS credential store | 🟢 Windows Credential Manager | ❔ |
| | Quality test set (26 cases, manual) | ✅ | ❔ |
| **Clean up selection** | Copy → clean → paste back | 🟢 | ⬜ |
| | Press again to undo while unchanged; editing resets | ✅ logic / 🟢 Windows | ⬜ |
| **Hotkeys** | Dictate (Ctrl+Alt+Space), Cancel (Esc, recording only), Clean up (Ctrl+Alt+C) | 🟢 | ⬜ |
| | Hold to talk / tap to toggle | ✅ | ⬜ |
| | Record a new shortcut ("Press new keys…") | ✅ logic / 🟢 UI | ⬜ |
| | Conflict warnings (Windows, common apps, other apps, Wirl's own) | ✅ | ⬜ |
| | Reset to defaults | 🟢 | ⬜ |
| **Pill** | Hidden when idle, never takes focus, always on top | 🟢 | ⬜ |
| | Follows the active window's monitor; bottom/top center or near mouse | ✅ placement / 🟢 Windows | ⬜ |
| | Recording / Processing / Inserted / Cleaned / Copied / Error states | ✅ | ⬜ |
| | Size (small/medium/large), status-messages toggle | ✅ | ⬜ |
| **Icons** | 25 provided icons, picker in icons.json order | ✅ (cut from `wirl-icons-sheet.png`) | ❔ |
| | Tray states: Ready, Recording, Processing, Off, Needs attention, Update ready | ✅ | ⬜ |
| **Dictionary** | Add, edit, delete (tombstones), search, alphabetical | ✅ | 🔵 |
| | Phonetic + case-insensitive matching | ✅ | ❔ |
| | Import (sync format or simple list) with merge; export | ✅ | ❔ |
| **Phone sync** | QR pairing, mDNS `_wirl._tcp`, AES-256-GCM, merge rule | ✅ (protocol in docs/sync-protocol.md) | 🔵 |
| | Paired phones with "Synced X min ago", Unpair | 🟢 | 🔵 |
| | Private networks only | ✅ | 🔵 |
| **App** | Tray menu: On/Off, Settings, Check for updates, Quit | 🟢 | ⬜ |
| | Settings window (Home, Dictation, AI cleanup, Dictionary, Hotkeys, Indicator, Setup, About) | 🟢 | ❔ |
| | Setup checklist, shown on first run; Test dictation | 🟢 | ❔ |
| | Start with Windows | 🟢 | ⬜ |
| | Reset to defaults (keeps key, dictionary, pairing) | ✅ | ❔ |
| | Developer options (7 clicks on version) with diagnostics | 🟢 | ❔ |
| **Reliability** | Timeouts on model load, transcription, API | ✅ | ❔ |
| | 30 s watchdog | ✅ | ❔ |
| | Diagnostics: last 20 entries with per-stage timing | ✅ | ❔ |
| **Updates** | Signed auto-updates from GitHub Releases, daily check, "Restart to update" | 🟢 (needs signing key, see README) | ⬜ |
| | CI on pull requests, automatic release on main | 🟢 | ⬜ |
