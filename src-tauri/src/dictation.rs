//! The dictation and clean-selection flows. Nothing heavy runs on the UI
//! thread: capture has its own thread, Whisper its own process, and
//! clipboard/keyboard work runs on blocking worker threads.

use crate::app::{self, App, Phase};
use crate::platform::{self, PasteReport, SystemInsert};
use crate::{audio, hotkeys, pill, tray};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use wirl_core::audio::{assemble, audio_ctx_for, Chunk, Chunker, ChunkerConfig};
use wirl_core::claude::CleanupError;
use wirl_core::diagnostics::{DiagEntry, DiagKind};
use wirl_core::hotkey::TriggerOutput;
use wirl_core::insert::{insert_text_then, AfterPaste, CleanDecision, InsertOutcome, PasteCheck, PasteError};
use wirl_core::ipc::initial_prompt;
use wirl_core::models::{self, Engine};
use wirl_core::pill::PillState;
use wirl_core::pipeline::{self, Final};
use wirl_core::prompt::{PromptMode, CLEANUP_MODEL};

/// Longest single dictation before Wirl stops on its own.
const MAX_RECORDING: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Stop,
    Cancel,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------- hotkey entry points ----------

pub fn on_dictate_pressed() {
    let a = app::get();
    let out = a.trigger.lock().pressed(now_ms());
    act(out);
}

pub fn on_dictate_released() {
    let a = app::get();
    let out = a.trigger.lock().released(now_ms());
    act(out);
}

fn act(out: TriggerOutput) {
    match out {
        TriggerOutput::Start => {
            tauri::async_runtime::spawn(async {
                if let Err(e) = start().await {
                    app::get().trigger.lock().reset();
                    log::warn!("start dictation: {e}");
                }
            });
        }
        TriggerOutput::Stop => stop(),
        TriggerOutput::None => {}
    }
}

pub fn stop() {
    if let Some(tx) = app::get().control.lock().as_ref() {
        let _ = tx.send(Control::Stop);
    }
}

pub fn cancel() {
    if let Some(tx) = app::get().control.lock().as_ref() {
        let _ = tx.send(Control::Cancel);
    }
}

/// Puts everything back to idle. Used at the end of every flow and by the watchdog.
pub fn reset_to_idle(a: &App) {
    a.session.fetch_add(1, Ordering::SeqCst);
    *a.phase.lock() = Phase::Idle;
    *a.control.lock() = None;
    a.trigger.lock().reset();
    a.watchdog.lock().idle();
    hotkeys::set_cancel_active(false);
    tray::refresh();
}

fn end_session(a: &App, session: u64) {
    if a.session_is(session) {
        reset_to_idle(a);
    }
}

fn show_error(msg: impl Into<String>) {
    pill::set(PillState::error(msg));
}

// ---------- dictation ----------

async fn start() -> Result<(), String> {
    let a = app::get().clone();
    let s = a.settings();
    if !s.enabled {
        return Err("off".into());
    }
    {
        let mut phase = a.phase.lock();
        if *phase != Phase::Idle {
            return Err("busy".into());
        }
        *phase = Phase::Recording;
    }
    let session = a.session.fetch_add(1, Ordering::SeqCst) + 1;
    // Where the text should go: the app in front when recording starts
    // (diagnostics compare it with the app in front when pasting).
    let target = tauri::async_runtime::spawn_blocking(platform::foreground_app)
        .await
        .ok()
        .flatten();
    let model_path = a.model_path(s.model);
    if !a.model_ready(s.model) {
        reset_to_idle(&a);
        show_error(format!("Download the {} model first", models::info(s.model).name));
        tray::refresh();
        return Err("no model".into());
    }
    // Make sure the model is (still) loaded; this is quick when it is.
    let sc = a.sidecar.clone();
    let mp = model_path.to_string_lossy().to_string();
    let engine = models::info(s.model).engine;
    let lang = models::language_code(s.model);
    tauri::async_runtime::spawn(async move {
        if let Err(e) = sc.load(engine, &mp, lang).await {
            log::error!("model load: {e}");
        }
    });
    // Warm the HTTPS connection to Claude while the user talks.
    if s.cleanup_enabled {
        let c = a.claude.clone();
        tauri::async_runtime::spawn(async move { c.warm().await });
    }
    let media = if s.pause_media {
        Some(tauri::async_runtime::spawn_blocking(platform::pause_playing_media))
    } else {
        None
    };
    let (audio_tx, audio_rx) = mpsc::unbounded_channel();
    let rec = tauri::async_runtime::spawn_blocking(move || audio::start(audio_tx))
        .await
        .map_err(|e| e.to_string())?;
    let media_token = match media {
        Some(h) => h.await.ok(),
        None => None,
    };
    let rec = match rec {
        Ok(r) => r,
        Err(e) => {
            if let Some(t) = media_token {
                tauri::async_runtime::spawn_blocking(move || platform::resume_media(t));
            }
            end_session(&a, session);
            show_error(e.clone());
            a.log_diag(DiagEntry {
                error: Some(e.clone()),
                ..DiagEntry::new(DiagKind::Error, "Microphone")
            });
            return Err(e);
        }
    };
    if !a.session_is(session) {
        rec.stop();
        return Err("reset".into());
    }
    if s.chime {
        audio::play_chime(true);
    }
    let (ctl_tx, ctl_rx) = mpsc::unbounded_channel();
    *a.control.lock() = Some(ctl_tx);
    hotkeys::set_cancel_active(true);
    pill::set(PillState::recording());
    tray::refresh();
    tauri::async_runtime::spawn(run_session(
        a.clone(),
        session,
        rec,
        audio_rx,
        ctl_rx,
        media_token,
        target,
    ));
    Ok(())
}

struct ChunkResult {
    index: usize,
    text: String,
    ms: u64,
    error: Option<String>,
    /// Length of the piece, for Diagnostics.
    dur_ms: usize,
    /// Not transcribed: no speech or voice at all in it.
    skipped: bool,
    /// Only faint speech: transcribed, text kept only if it's real words.
    faint: bool,
}

async fn transcribe_worker(
    a: Arc<App>,
    mut rx: mpsc::UnboundedReceiver<Chunk>,
    language: String,
    prompt: String,
) -> Vec<ChunkResult> {
    let mut out = Vec::new();
    while let Some(chunk) = rx.recv().await {
        let dur_ms = chunk.duration_ms();
        if !chunk.has_speech && !chunk.weak_speech {
            out.push(ChunkResult {
                index: chunk.index,
                text: String::new(),
                ms: 0,
                error: None,
                dur_ms,
                skipped: true,
                faint: false,
            });
            continue;
        }
        let faint = !chunk.has_speech;
        let ctx = audio_ctx_for(chunk.samples.len());
        let timeout = Duration::from_millis(pipeline::CHUNK_TRANSCRIBE_TIMEOUT_MS);
        match a
            .sidecar
            .transcribe(&chunk.samples, &language, &prompt, ctx, timeout)
            .await
        {
            Ok((text, ms)) => out.push(ChunkResult {
                index: chunk.index,
                // A faint piece keeps its text only if it's real words.
                text: if faint {
                    pipeline::weak_chunk_text(&text).unwrap_or_default()
                } else {
                    text
                },
                ms,
                error: None,
                dur_ms,
                skipped: false,
                faint,
            }),
            Err(e) => out.push(ChunkResult {
                index: chunk.index,
                text: String::new(),
                ms: 0,
                error: Some(e),
                dur_ms,
                skipped: false,
                faint,
            }),
        }
    }
    out
}

async fn run_session(
    a: Arc<App>,
    session: u64,
    rec: audio::Recording,
    mut audio_rx: mpsc::UnboundedReceiver<Vec<f32>>,
    mut ctl_rx: mpsc::UnboundedReceiver<Control>,
    media: Option<platform::MediaPauseToken>,
    target: Option<platform::AppInfo>,
) {
    let s = a.settings();
    let started = Instant::now();
    let mut chunker = Chunker::new(ChunkerConfig::default());
    // Whisper initial prompt: dictionary terms.
    let terms: Vec<String> = a.dict.lock().visible().iter().map(|e| e.term.clone()).collect();
    let term_refs: Vec<&str> = terms.iter().map(|t| t.as_str()).collect();
    let prompt = initial_prompt(&term_refs);
    // The model decides the language ("en", "yue" or "zh").
    let language = models::language_code(s.model).to_string();
    let (chunk_tx, chunk_rx) = mpsc::unbounded_channel::<Chunk>();
    let worker = tauri::async_runtime::spawn(transcribe_worker(a.clone(), chunk_rx, language, prompt));
    let mut last_level = Instant::now();
    let mut chunk_count = 0usize;

    let control = loop {
        tokio::select! {
            c = ctl_rx.recv() => break c.unwrap_or(Control::Cancel),
            block = audio_rx.recv() => {
                let Some(block) = block else { break Control::Stop };
                for c in chunker.push(&block) {
                    chunk_count += 1;
                    let _ = chunk_tx.send(c);
                }
                if last_level.elapsed() >= Duration::from_millis(40) {
                    pill::level(chunker.level());
                    last_level = Instant::now();
                }
            }
            _ = tokio::time::sleep(MAX_RECORDING.saturating_sub(started.elapsed())) => break Control::Stop,
        }
        if !a.session_is(session) {
            break Control::Cancel;
        }
    };

    // Recording is over: stop the mic, resume media, take the final chunk.
    hotkeys::set_cancel_active(false);
    tauri::async_runtime::spawn_blocking(move || rec.stop()).await.ok();
    while let Ok(block) = audio_rx.try_recv() {
        for c in chunker.push(&block) {
            chunk_count += 1;
            let _ = chunk_tx.send(c);
        }
    }
    if let Some(t) = media {
        tauri::async_runtime::spawn_blocking(move || platform::resume_media(t));
    }
    if s.chime {
        audio::play_chime(false);
    }
    let duration_ms = chunker.total_ms();
    let any_speech = chunker.any_speech();

    if control == Control::Cancel || !a.session_is(session) {
        drop(chunk_tx);
        worker.abort();
        pill::set(PillState::Hidden);
        end_session(&a, session);
        return;
    }
    if pipeline::discard_before_transcription(duration_ms, any_speech) {
        drop(chunk_tx);
        worker.abort();
        pill::set(PillState::Hidden);
        end_session(&a, session);
        return;
    }
    // The final chunk starts transcribing the instant recording stops.
    if let Some(last) = chunker.finish() {
        chunk_count += 1;
        let _ = chunk_tx.send(last);
    }
    drop(chunk_tx);
    *a.phase.lock() = Phase::Processing;
    a.watchdog.lock().busy(now_ms(), "processing");
    pill::set(PillState::Processing);
    tray::refresh();

    let stop_at = Instant::now();
    let results = worker.await.unwrap_or_default();
    let tail_ms = stop_at.elapsed().as_millis() as u64;
    if !a.session_is(session) {
        return;
    }
    let mut diag = DiagEntry::new(
        DiagKind::Dictation,
        format!("{:.1} s dictation, {} chunks", duration_ms as f64 / 1000.0, chunk_count),
    );
    let model = models::info(s.model);
    let engine_name = match model.engine {
        Engine::Whisper => "Whisper",
        Engine::SenseVoice => "SenseVoice",
    };
    diag.model = Some(model.name.to_string());
    for r in results.iter().filter(|r| !r.skipped) {
        diag.push_stage(format!("{engine_name} chunk {}", r.index + 1), r.ms);
    }
    diag.push_stage(format!("{engine_name} after stop"), tail_ms);
    // What happened to each piece that wasn't plain speech, so lost words
    // show up in Diagnostics instead of only as a missing chunk number.
    for r in &results {
        let piece = format!("Piece {} ({:.1} s)", r.index + 1, r.dur_ms as f64 / 1000.0);
        if let Some(e) = &r.error {
            diag.notes
                .push(format!("{piece}: couldn't be transcribed ({e}); its words are missing"));
        } else if r.skipped {
            diag.notes.push(format!("{piece}: no speech, skipped"));
        } else if r.faint && r.text.is_empty() {
            diag.notes.push(format!("{piece}: only faint sound, no words in it"));
        } else if r.faint {
            diag.notes.push(format!("{piece}: faint speech, kept: {}", r.text));
        }
    }
    let errors: Vec<String> = results.iter().filter_map(|r| r.error.clone()).collect();
    let parts: Vec<(usize, String)> = results.iter().map(|r| (r.index, r.text.clone())).collect();
    let joined = assemble(&parts);
    let Some(raw) = pipeline::transcript_or_none(&joined) else {
        if !errors.is_empty() {
            diag.error = Some(errors.join("; "));
            a.log_diag(DiagEntry {
                kind: DiagKind::Error,
                ..diag
            });
            show_error("Couldn't transcribe. Try again.");
        } else {
            pill::set(PillState::Hidden);
        }
        end_session(&a, session);
        return;
    };
    diag.heard = Some(raw.clone());
    // Noise or a stray sound (fewer than 3 letters): nothing is sent to
    // Claude, nothing is inserted, nothing is shown. Same rule as Android.
    if !pipeline::has_words(&raw) {
        diag.summary = format!("{} · no words, nothing inserted", diag.summary);
        diag.notes.push(format!(
            "Heard fewer than {} letters, so it was treated as noise: Claude wasn't asked and nothing was inserted.",
            pipeline::MIN_LETTERS
        ));
        pill::set(PillState::Hidden);
        a.log_diag(diag);
        end_session(&a, session);
        return;
    }

    // The model decides the cleanup mode (standard, Cantonese, Mandarin).
    let mode = model.language.prompt_mode();
    let entries_owned: Vec<_> = {
        let d = a.dict.lock();
        match mode {
            PromptMode::Cantonese | PromptMode::Mandarin => d.matching_chinese(&raw),
            PromptMode::Standard => d.matching(&raw),
        }
        .into_iter()
        .cloned()
        .collect()
    };
    let terms: Vec<&str> = entries_owned.iter().map(|e| e.term.as_str()).collect();
    // Short, clean English from Whisper (which punctuates) is typed as heard:
    // cleanup adds little there and can answer a short question instead of
    // typing it. Same rule as Android.
    let as_heard = s.cleanup_enabled
        && mode == PromptMode::Standard
        && model.engine == Engine::Whisper
        && pipeline::types_as_heard(&raw, !entries_owned.is_empty());
    if as_heard {
        diag.notes.push("Short and clean, typed as heard (no cleanup).".into());
    }

    // Claude cleanup (or raw text if cleanup is off or not needed).
    let cleanup = if s.cleanup_enabled && !as_heard {
        diag.thinking = Some(false);
        let key = a.api_key.read().clone().unwrap_or_default();
        let entries: Vec<&_> = entries_owned.iter().collect();
        let r = a
            .claude
            .cleanup(
                &key,
                mode,
                &raw,
                &s.style_notes,
                &entries,
                Duration::from_millis(pipeline::CLEANUP_TIMEOUT_MS),
            )
            .await;
        match &r {
            Ok(c) => {
                diag.push_stage("Claude first byte", c.first_byte_ms);
                diag.push_stage("Claude total", c.total_ms);
                diag.cleaned = Some(c.text.clone());
                diag.connection = Some(connection_label(c.warm_connection));
                diag.model = Some(format!(
                    "{} + {}",
                    diag.model.clone().unwrap_or_default(),
                    CLEANUP_MODEL
                ));
            }
            Err(e) => {
                diag.error = Some(e.to_string());
                if e.needs_attention() {
                    tray::refresh();
                }
            }
        }
        Some(r.map(|c| c.text))
    } else {
        None
    };
    if !a.session_is(session) {
        return;
    }
    if let Some(Ok(t)) = &cleanup {
        if pipeline::is_reply_to_speaker(&raw, t, &terms) {
            diag.notes.push(
                "Claude answered instead of cleaning up (what you said sounded like a request), so your own words were inserted."
                    .into(),
            );
        }
    }
    match pipeline::after_cleanup(&raw, cleanup, &terms) {
        Final::Discard => {
            diag.summary = format!("{} · nothing inserted", diag.summary);
            diag.notes.push(
                "Claude's reply had no words (empty, only punctuation, or a remark about the recording), so nothing was inserted."
                    .into(),
            );
            pill::set(PillState::Hidden);
        }
        Final::Insert { text, warning } => {
            diag.inserted = Some(text.clone());
            let t = Instant::now();
            let (outcome, report) = paste_with_report(text).await;
            diag.push_stage("Insert", t.elapsed().as_millis() as u64);
            diag.push_stage("Stop to insert", stop_at.elapsed().as_millis() as u64);
            add_paste_notes(&mut diag, target.as_ref(), &report, &outcome);
            report_insert(&mut diag, &outcome);
            let pasted = matches!(outcome, InsertOutcome::Pasted { .. });
            let tip = pasted && record_dictation(&a);
            match (outcome, warning) {
                (InsertOutcome::Pasted { .. }, None) if tip => pill::set(PillState::inserted_with_clean_tip(
                    &s.hotkeys.clean_selection.to_string(),
                )),
                (InsertOutcome::Pasted { .. }, None) => pill::set(PillState::Inserted),
                (InsertOutcome::Pasted { .. }, Some(w)) => show_error(w),
                (InsertOutcome::CopiedOnly { .. }, _) => pill::set(PillState::Copied),
                (InsertOutcome::Failed(e), _) => {
                    diag.error = Some(e);
                    show_error("Couldn't insert the text");
                }
            }
        }
    }
    a.log_diag(diag);
    end_session(&a, session);
}

/// Counts a successful dictation for onboarding. Returns true if the
/// one-time "try clean-up" tip should show now.
fn record_dictation(a: &App) -> bool {
    let tip = {
        let mut s = a.settings.write();
        let can_show = s.status_messages;
        s.usage.record_dictation(can_show)
    };
    a.save_settings();
    a.notify();
    tip
}

fn record_cleanup(a: &App) {
    a.settings.write().usage.record_cleanup();
    a.save_settings();
    a.notify();
}

/// Pastes `text` and reports what was around the paste (for Diagnostics).
/// With "Keep what you dictate on the clipboard" on (default), the text
/// stays on the clipboard so a paste that didn't land can be repeated.
async fn paste_with_report(text: String) -> (InsertOutcome, PasteReport) {
    let keep = app::get().settings().keep_on_clipboard;
    tauri::async_runtime::spawn_blocking(move || {
        let mut p = SystemInsert::new(keep);
        let after = if keep { AfterPaste::Keep } else { AfterPaste::Restore };
        let outcome = insert_text_then(&mut p, &text, after);
        p.report.other_apps = platform::running_dictation_apps();
        (outcome, p.report)
    })
    .await
    .unwrap_or_else(|e| (InsertOutcome::Failed(e.to_string()), PasteReport::default()))
}

/// Diagnostics notes for one paste: how much was pasted, into which app
/// (and whether it's the app the dictation started in), keys still held,
/// and other dictation apps running. Program names only, never window titles.
fn add_paste_notes(
    diag: &mut DiagEntry,
    started_in: Option<&platform::AppInfo>,
    report: &PasteReport,
    outcome: &InsertOutcome,
) {
    if let Some(text) = &diag.inserted {
        let chars = text.chars().count();
        let visible = text.chars().filter(|c| c.is_alphanumeric()).count();
        diag.notes.push(format!(
            "Pasted {chars} character{} ({visible} letters or digits)",
            if chars == 1 { "" } else { "s" }
        ));
    }
    match (started_in, &report.app) {
        (Some(s), Some(p)) if s.window == p.window => diag
            .notes
            .push(format!("Pasted into {}, the window you started in", p.program)),
        (Some(s), Some(p)) => diag.notes.push(format!(
            "Pasted into {}, but you started in {} (the window in front changed)",
            p.program, s.program
        )),
        (None, Some(p)) => diag.notes.push(format!("Pasted into {}", p.program)),
        (_, None) => diag.notes.push("No window was in front when pasting".into()),
    }
    if !report.keys_held.is_empty() {
        diag.notes.push(format!(
            "Keys still held when pasting: {} (Ctrl+V may have become another shortcut)",
            report.keys_held.join(", ")
        ));
    }
    if !report.other_apps.is_empty() {
        diag.notes.push(format!(
            "Also running: {} (it may use the same keys or the clipboard)",
            report.other_apps.join(", ")
        ));
    }
    if let Some(p) = &report.clipboard_held_by {
        diag.notes
            .push(format!("The clipboard was busy ({p} had it open), so Wirl waited"));
    }
    if let Some(n) = check_note(outcome, report.checkable) {
        diag.notes.push(n.into());
    }
}

/// What the check of the text box found, in plain words.
fn check_note(outcome: &InsertOutcome, checkable: bool) -> Option<&'static str> {
    Some(match outcome {
        InsertOutcome::Pasted {
            check: PasteCheck::Landed,
            ..
        } => "Checked the text box: the text appeared",
        InsertOutcome::Pasted {
            check: PasteCheck::LandedOnRetry,
            ..
        } => "Checked the text box: the first Ctrl+V didn't land, the second one did",
        InsertOutcome::Pasted {
            check: PasteCheck::Unchecked,
            ..
        } if checkable => "Couldn't confirm the text appeared (the focus moved or the box stopped answering)",
        InsertOutcome::Pasted { .. } => "This app's text box can't be read, so the paste wasn't checked",
        InsertOutcome::CopiedOnly {
            reason: PasteError::NotLanded,
        } => "Checked the text box: nothing appeared after two Ctrl+V, so the text was left on the clipboard",
        _ => return None,
    })
}

/// Diagnostics wording for whether Claude's connection was already open.
fn connection_label(warm: bool) -> String {
    if warm {
        "warm (already open)".into()
    } else {
        "new (opened for this request)".into()
    }
}

fn report_insert(diag: &mut DiagEntry, outcome: &InsertOutcome) {
    diag.insert_method = Some(match outcome {
        InsertOutcome::Pasted { restored: true, .. } => "Clipboard paste (clipboard restored)".into(),
        InsertOutcome::Pasted { restored: false, .. } if app::get().settings().keep_on_clipboard => {
            "Clipboard paste (text kept on the clipboard)".into()
        }
        InsertOutcome::Pasted { restored: false, .. } => "Clipboard paste".into(),
        InsertOutcome::CopiedOnly {
            reason: PasteError::Blocked,
        } => "Copied only (paste blocked)".into(),
        InsertOutcome::CopiedOnly {
            reason: PasteError::NotLanded,
        } => "Copied only (the paste didn't land)".into(),
        InsertOutcome::CopiedOnly { reason } => format!("Copied only ({reason:?})"),
        InsertOutcome::Failed(e) => format!("Failed: {e}"),
    });
}

// ---------- quick switch ----------

/// Switch speech model hotkey: moves to the next model ticked in Settings ›
/// Dictation (same rules as Android's swipe on the bubble) and shows its tag
/// on the pill. While recording, what was said so far is dropped, as with
/// Cancel, and recording starts again in the new model.
pub fn on_switch_model() {
    tauri::async_runtime::spawn(async {
        if let Err(e) = switch_model().await {
            log::warn!("switch model: {e}");
        }
    });
}

async fn switch_model() -> Result<(), String> {
    let a = app::get().clone();
    let s = a.settings();
    if !s.enabled {
        return Err("off".into());
    }
    let phase = a.phase();
    if matches!(phase, Phase::Processing | Phase::CleaningSelection) {
        // Finishing the last dictation or clean-up; switching now would be confusing.
        return Err("busy".into());
    }
    let ready = |m| a.model_ready(m);
    if !models::cycle::can_switch(&s.quick_switch, s.model, ready) {
        let m = models::info(s.model);
        if phase == Phase::Idle {
            pill::set(PillState::model_switched(m.tag, m.name, true));
        }
        return Ok(());
    }
    let next = models::cycle::step(&s.quick_switch, s.model, ready, 1);
    let info = models::info(next);
    a.settings.write().model = next;
    a.save_settings();
    a.log_diag(DiagEntry::new(DiagKind::Info, format!("Switched to {}", info.name)));
    crate::commands::ensure_model(next);
    a.notify();
    tray::refresh();
    if phase != Phase::Recording {
        pill::set(PillState::model_switched(info.tag, info.name, false));
        return Ok(());
    }
    // Recording: drop what was said, then record again in the new model.
    // Keep the Dictate key's state, so a held key still stops on release.
    let trigger = a.trigger.lock().clone();
    cancel();
    let deadline = Instant::now() + Duration::from_secs(3);
    while a.phase() != Phase::Idle {
        if Instant::now() > deadline {
            return Err("the old recording didn't stop".into());
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    start().await?;
    *a.trigger.lock() = trigger;
    pill::flash_switched_recording(info.tag, info.name);
    Ok(())
}

// ---------- clean up selection ----------

pub fn on_clean_selection() {
    let a = app::get().clone();
    if !a.settings().enabled {
        return;
    }
    {
        let mut phase = a.phase.lock();
        if *phase != Phase::Idle {
            return;
        }
        *phase = Phase::CleaningSelection;
    }
    let session = a.session.fetch_add(1, Ordering::SeqCst) + 1;
    a.watchdog.lock().busy(now_ms(), "clean selection");
    tauri::async_runtime::spawn(async move {
        clean_selection(a.clone(), session).await;
        end_session(&a, session);
    });
}

async fn paste(text: String) -> InsertOutcome {
    paste_with_report(text).await.0
}

async fn clean_selection(a: Arc<App>, session: u64) {
    let s = a.settings();
    let mut diag = DiagEntry::new(DiagKind::CleanSelection, "Clean up selection");
    let t = Instant::now();
    let selection = tauri::async_runtime::spawn_blocking(platform::copy_selection)
        .await
        .ok()
        .flatten();
    diag.push_stage("Copy selection", t.elapsed().as_millis() as u64);
    let mut decision = a.undo.lock().on_hotkey(selection.as_deref());
    if let CleanDecision::Reselect { chars } = decision {
        let reselected = tauri::async_runtime::spawn_blocking(move || {
            if platform::select_back(chars).is_err() {
                return None;
            }
            platform::copy_selection()
        })
        .await
        .ok()
        .flatten();
        decision = a.undo.lock().after_reselect(reselected.as_deref());
        if decision == CleanDecision::NothingSelected {
            tauri::async_runtime::spawn_blocking(platform::collapse_selection_right)
                .await
                .ok();
        }
    }
    if !a.session_is(session) {
        return;
    }
    match decision {
        CleanDecision::NothingSelected | CleanDecision::Reselect { .. } => {
            show_error("Select some text first");
        }
        CleanDecision::Undo { original } => {
            let outcome = paste(original).await;
            report_insert(&mut diag, &outcome);
            a.undo.lock().record_undo();
            diag.summary = "Undo clean up".into();
            match outcome {
                InsertOutcome::Pasted { .. } => pill::set(PillState::Undone),
                InsertOutcome::CopiedOnly { .. } => pill::set(PillState::Copied),
                InsertOutcome::Failed(_) => show_error("Couldn't paste the text"),
            }
            a.log_diag(diag);
        }
        CleanDecision::Clean(original) => {
            pill::set(PillState::Processing);
            tray::refresh();
            let key = a.api_key.read().clone().unwrap_or_default();
            let entries_owned: Vec<_> = a.dict.lock().matching(&original).into_iter().cloned().collect();
            let entries: Vec<&_> = entries_owned.iter().collect();
            diag.thinking = Some(false);
            diag.model = Some(CLEANUP_MODEL.into());
            diag.heard = Some(original.clone());
            let r = a
                .claude
                .cleanup(
                    &key,
                    PromptMode::Standard,
                    &original,
                    &s.style_notes,
                    &entries,
                    Duration::from_millis(pipeline::CLEANUP_TIMEOUT_MS),
                )
                .await;
            if !a.session_is(session) {
                return;
            }
            match r {
                Ok(c)
                    if pipeline::is_reply_to_speaker(
                        &original,
                        &c.text,
                        &entries_owned.iter().map(|e| e.term.as_str()).collect::<Vec<_>>(),
                    ) =>
                {
                    diag.cleaned = Some(c.text.clone());
                    diag.notes.push(
                        "Claude answered instead of cleaning up (the text sounded like a request), so the selection was left as it was."
                            .into(),
                    );
                    show_error("Couldn't clean this up");
                }
                Ok(c) if !c.text.trim().is_empty() => {
                    diag.push_stage("Claude first byte", c.first_byte_ms);
                    diag.push_stage("Claude total", c.total_ms);
                    diag.connection = Some(connection_label(c.warm_connection));
                    diag.cleaned = Some(c.text.clone());
                    diag.inserted = Some(c.text.clone());
                    let cleaned = c.text.clone();
                    let t = Instant::now();
                    let (outcome, report) = paste_with_report(cleaned.clone()).await;
                    diag.push_stage("Insert", t.elapsed().as_millis() as u64);
                    add_paste_notes(&mut diag, None, &report, &outcome);
                    report_insert(&mut diag, &outcome);
                    match outcome {
                        InsertOutcome::Pasted { .. } => {
                            a.undo.lock().record(&original, &cleaned);
                            record_cleanup(&a);
                            pill::set(PillState::cleaned(&s.hotkeys.clean_selection.to_string()));
                        }
                        InsertOutcome::CopiedOnly { .. } => pill::set(PillState::Copied),
                        InsertOutcome::Failed(_) => show_error("Couldn't paste the text"),
                    }
                }
                Ok(_) => show_error("Nothing to clean up"),
                Err(e) => {
                    diag.error = Some(e.to_string());
                    let msg = match e {
                        CleanupError::NoKey => "Add your API key in Setup".to_string(),
                        other => other.selection_message(),
                    };
                    show_error(msg);
                    tray::refresh();
                }
            }
            a.log_diag(diag);
        }
    }
}

// ---------- watchdog ----------

/// Resets internal state if anything has been stuck for over 30 seconds.
pub fn start_watchdog() {
    tauri::async_runtime::spawn(async {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            let a = app::get();
            let stuck = a.watchdog.lock().check(now_ms()).map(|s| s.to_string());
            if let Some(stage) = stuck {
                log::error!("watchdog: stuck in {stage}; resetting");
                reset_to_idle(a);
                pill::set(PillState::error("Something got stuck. Wirl reset itself."));
                a.log_diag(DiagEntry {
                    error: Some(format!("Watchdog reset after 30 s in {stage}")),
                    ..DiagEntry::new(DiagKind::Error, "Watchdog reset")
                });
            }
        }
    });
}
