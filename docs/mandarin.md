# Mandarin mode

Wirl can dictate Mandarin Chinese, often mixed with English, written in **simplified characters** (mainland China style). This file holds the cleanup instructions for Mandarin, so Wirl for Windows and Wirl for Android send Claude exactly the same text. Cantonese works the same way; see `docs/cantonese.md`.

## How it works (Windows)

- **Speech to text.** Under *Dictation › Speech model*, **Mandarin (普通话)** runs SenseVoice through sherpa-onnx in the `wirl-sensevoice` helper program, with the same model files as Cantonese (one 166 MB download serves both), told the language `zh` instead of `yue`. `use_itn` is on. Details of the model, the download and the engine are in `docs/cantonese.md`.
- **The model decides the language.** There is no separate language setting: picking Mandarin means the engine is told `zh` and cleanup uses Mandarin mode.
- **Cleanup.** When the Mandarin model produced the transcript, the block below is added to the built-in core prompt, after it and before the speaker's style notes. Everything else stays the same. SenseVoice writes simplified characters without punctuation and English in capitals; Wirl lower-cases the capitals, and this block makes the final text. Clean-up of selected text doesn't use it.

## Checking quality (manual, your own key)

`tests/quality/cases.json` has Mandarin cleanup cases (names starting with "Mandarin:"). Run them by hand; they never run on GitHub:

```powershell
$env:ANTHROPIC_API_KEY = "sk-ant-..."
cargo run -p wirl-core --example cleanup_quality -- mandarin
```

## The block

**Keep this identical in both apps.** The desktop copy is `MANDARIN_BLOCK` in `crates/wirl-core/src/prompt.rs`, and a unit test checks it matches this file exactly. On Android it's `CleanupPrompt.MANDARIN_MODE` (tobijeh/Wirlwind, `docs/mandarin.md`).

```text
--- MANDARIN MODE ---
The speaker is talking Mandarin Chinese, often mixed with English.
- Write in natural written Mandarin as the speaker said it, in simplified characters (mainland China style).
- Never use traditional characters, and never use Cantonese words or characters (such as 佢, 嘅, 唔, 咗, 喺).
- Keep English words and phrases in English, exactly where they were spoken.
- Fix characters the speech recognizer got wrong, where a character with the same sound (同音字) clearly makes more sense.
- Remove filler sounds and words (such as 嗯, 呃, 那个, 就是说) and false starts. Keep sentence-final particles (such as 吧, 呢, 啊, 嘛) when they carry meaning or tone.
- Use full-width Chinese punctuation (，。？！：) in Chinese text.
- Polish lightly. The result should read like the speaker typed it themselves in a message.
- Apply the personal dictionary as usual.
--- END MANDARIN MODE ---
```
