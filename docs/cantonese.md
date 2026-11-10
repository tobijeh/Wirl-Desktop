# Cantonese mode

Whisper handles Cantonese badly, so Wirl has a second speech engine for it: **SenseVoice**, run on the computer through [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx). Pick **Cantonese (粵語)** under *Dictation › Speech model*. Everything else (hotkeys, hold to talk, cancel, clean-up and undo, the pill, the chime, AI cleanup, the personal dictionary, phone sync) works the same.

## How it works

1. **Model.** The official sherpa-onnx release `sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09` (int8), from the k2-fsa/sherpa-onnx GitHub releases (`asr-models`). It is downloaded inside the app the first time the model is picked (166 MB), checked against its SHA-256, and unpacked into the models folder (`model.int8.onnx` and `tokens.txt`, 237 MB). It is not in the installer.
2. **Engine.** SenseVoice runs in its own helper program (`wirl-sensevoice`), like Whisper (`wirl-whisper`). sherpa-onnx and onnxruntime are linked into it statically, so no DLLs are needed. If it crashes or takes too long, Wirl stops it, shows an error in the pill, and starts it again; the app and hotkeys keep working.
3. **Language.** SenseVoice gets the language `yue`. Tested against `auto` on the model's Cantonese test clips and on Cantonese mixed with English: the results were the same or slightly better, and short clips can't be mistaken for Mandarin. English words mixed into Cantonese come through in English.
4. **Punctuation.** `use_itn` (punctuation and number formatting) is turned on. This Cantonese model mostly returns text without punctuation either way; the AI cleanup adds it.
5. **Long dictations.** Wirl cuts the recording at natural pauses into 5–10 s pieces while you talk (the same chunker used for Whisper), transcribes each one in the background, and joins them in order. Pieces are joined without spaces between Chinese characters.
6. **Text clean-up before AI.** SenseVoice writes simplified characters and English in capitals. Wirl turns all-capital English words into lower case; the AI cleanup turns the rest into Hong Kong-style traditional characters.
7. **AI cleanup.** When the Cantonese model produced the transcript, the block below is added right after the built-in core prompt, before the user's style notes and dictionary. Dictionary entries written in Chinese characters are always sent along in this mode (the model writes simplified characters, entries are usually typed in traditional ones).
8. **Inserting.** Text goes in through the clipboard (Unicode) and Ctrl+V, then the clipboard is put back, so Chinese characters paste correctly into any app.

## The Cantonese block

**This text must stay identical in the Android app (tobijeh/Wirlwind) and the desktop app (tobijeh/Wirl-Desktop).** Change both together. The desktop copy is `CANTONESE_BLOCK` in `crates/wirl-core/src/prompt.rs`; a unit test checks it matches this file.

```text
--- CANTONESE MODE ---
The speaker is talking Cantonese, often mixed with English.
- Write exactly as spoken, in written spoken Cantonese (口語), in traditional characters (Hong Kong style). For example: 佢, 嘅, 唔, 咗, 喺, 啲, 咁, 冇, 嚟, 睇.
- Never rewrite into formal written Chinese (書面語) or Mandarin phrasing, and never use simplified characters.
- Keep English words and phrases in English, exactly where they were spoken.
- If the transcript uses Mandarin-style characters where the speaker clearly said Cantonese, correct them to the Cantonese characters.
- Remove pure filler sounds (such as 呃 and 嗯) and false starts. Keep sentence-final particles (such as 呀, 啦, 喎, 囉, 嘛, 㗎, 啩) because they carry meaning and tone.
- Use full-width Chinese punctuation (，。？！：) in Chinese text.
- Polish lightly. The result should read like the speaker typed it themselves in a message.
- Apply the personal dictionary as usual.
--- END CANTONESE MODE ---
```

## Checking it by hand

- `wirl-sensevoice --model <model folder> [--language yue] [--no-itn] <file.wav>...` prints what SenseVoice hears in 16 kHz mono WAV files (the model archive has test clips in `test_wavs/`).
- `cargo run -p wirl-core --example cleanup_quality -- cantonese` runs the Cantonese cleanup samples in `tests/quality/cases.json` (needs `ANTHROPIC_API_KEY` in your shell; never run in CI).
