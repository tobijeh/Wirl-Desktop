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
| | Quiet voices and trailing-off last words kept (room-relative voice detection) | ✅ | ❔ |
| | Final chunk starts the instant recording stops | 🟢 | ❔ |
| | `audio_ctx` sized per chunk (min 384) | ✅ | ❔ |
| | Dictionary terms as Whisper initial prompt | ✅ | ❔ |
| | Sound-tag and lone-hallucination filtering | ✅ | ❔ |
| | Silent discard (< 1 s or no speech) | ✅ | ❔ |
| | Insert at cursor via clipboard paste, clipboard restored | ✅ logic / 🟢 Windows | ❔ |
| | "Copied, press Ctrl+V" fallback when paste is blocked | ✅ logic / 🟢 Windows | ❔ |
| | Pause media during dictation (Windows media controls) | 🟢 | ❔ |
| | Start/stop chime | 🟢 | ❔ |
| **Models** | One Speech model list, one model per language: English · Fast (Whisper base.en q5_1), English · Accurate (Whisper small.en), Cantonese (粵語), Mandarin (普通话). The model decides the language and the cleanup mode; no separate language setting | ✅ | 🔵 |
| | Multilingual removed; settings migration (version 2) moves its users to English · Fast and deletes its file | ✅ | 🔵 |
| | On-demand download with size, progress, Cancel (keeps the partial file, Resume continues), SHA-256 check, Delete | ✅ | 🔵 |
| | Amber "not downloaded yet" note on the chosen model's row | ✅ | 🔵 |
| | Quick switch: tick several downloaded models; the Switch speech model hotkey (Ctrl+Alt+M) moves between them, even while recording (restarts in the new model), and the pill shows the tag (EN, EN+, 粵, 普) for a moment; clicking a row never downloads, "Use now" picks the model | ✅ logic / 🟢 Windows | 🔵 swipe on the bubble (#62) |
| | Active model kept loaded | 🟢 | ❔ |
| | **Cantonese (粵語)** and **Mandarin (普通话)**: SenseVoice through sherpa-onnx 2.0.1 in its own sidecar, language `yue` / `zh`, mixed with English (docs/cantonese.md, docs/mandarin.md) | ✅ logic, verified on Linux / 🟢 Windows | 🔵 |
| | One SenseVoice download (166 MB, size + SHA-256, unpacked with sizes checked) serves both; Delete removes both after a warning | ✅ | 🔵 |
| **AI cleanup** | Claude Haiku 5.5, thinking off, exact built-in prompt | ✅ | 🔵 same app family |
| | Personal style notes | ✅ | ❔ |
| | Only plausibly matching dictionary entries sent | ✅ | ❔ |
| | Keep-alive client, connection warmed at record start | 🟢 | ❔ |
| | Raw-text fallback with plain reason | ✅ | ❔ |
| | Line breaks and lists preserved | ✅ | ❔ |
| | Own API key, stored in the OS credential store | 🟢 Windows Credential Manager | ❔ |
| | Cantonese and Mandarin mode blocks added to the prompt (same text as Android, tests check docs match) | ✅ | 🔵 |
| | Quality test set (38 cases incl. 6 Cantonese and 6 Mandarin, manual) | ✅ | 🔵 |
| **Clean up selection** | Copy → clean → paste back | 🟢 | ⬜ |
| | Press again to undo while unchanged; editing resets | ✅ logic / 🟢 Windows | ⬜ |
| **Hotkeys** | Dictate (Ctrl+Alt+Space), Cancel (Esc, recording only), Clean up (Ctrl+Alt+C), Switch speech model (Ctrl+Alt+M) | 🟢 | ⬜ |
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
| **Phone sync** | Encrypted mailbox relay on Cloudflare, no accounts; works on any network (protocol v2 in docs/sync-protocol.md) | ✅ logic + relay tested / 🟢 needs a live check | ⬜ (Android update pending) |
| | QR pairing; several phones share one dictionary | ✅ | ⬜ |
| | Phones listed with "Synced X min ago"; Sync now | 🟢 | ⬜ |
| | Disconnect all phones (closes the mailbox, new code) | ✅ | ⬜ |
| | Syncs on launch, after changes, on opening the Dictionary, every 5 minutes | 🟢 | ⬜ |
| **Onboarding** | Welcome tour on first launch: try talking and tidying up in practice boxes, with the real hotkeys | ✅ logic / 🟢 Windows | ❔ |
| | Replay the tour from Setup or Home | ✅ | ❔ |
| | "Your two shortcuts" card on Home until each has been used 3 times | ✅ | ❔ |
| | One-time "try clean-up" tip in the pill | ✅ | ❔ |
| **App** | Tray menu: On/Off, Settings, Check for updates, Quit | 🟢 | ⬜ |
| | Settings window (Home, Dictation, AI cleanup, Dictionary, Hotkeys, Indicator, Setup, About) | 🟢 | ❔ |
| | Setup checklist, shown on first run; Test dictation | 🟢 | ❔ |
| | Start with Windows | 🟢 | ⬜ |
| | Reset to defaults (keeps key, dictionary, pairing) | ✅ | ❔ |
| | Developer options (7 clicks on version) with diagnostics | 🟢 | ❔ |
| **Reliability** | Timeouts on model load, transcription, API | ✅ | ❔ |
| | 30 s watchdog | ✅ | ❔ |
| | Diagnostics: last 20 entries with per-stage timing | ✅ | ❔ |
| **Updates** | Signed auto-updates from GitHub Releases, daily check, "Restart to update" | 🟢 (signing key set up; needs a check that an installed copy updates) | ⬜ |
| | CI on pull requests, automatic release on main | 🟢 | ⬜ |
