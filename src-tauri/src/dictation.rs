//! The dictation and clean-selection flows. Nothing heavy runs on the UI
//! thread: capture has its own thread, Whisper its own process, and
//! clipboard/keyboard work runs on blocking worker threads.

use crate::app::{self, App, Phase};
use crate::platform::{self, SystemInsert};
use crate::{audio, hotkeys, pill, tray};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use wirl_core::audio::{assemble, audio_ctx_for, Chunk, Chunker, ChunkerConfig};
use wirl_core::claude::CleanupError;
use wirl_core::diagnostics::{DiagEntry, DiagKind};
use wirl_core::hotkey::TriggerOutput;
use wirl_core::insert::{insert_text, CleanDecision, InsertOutcome, PasteError};
use wirl_core::ipc::initial_prompt;
use wirl_core::models;
use wirl_core::pill::PillState;
use wirl_core::pipeline::{self, Final};
use wirl_core::prompt::CLEANUP_MODEL;

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
    let model_path = a.model_path(s.model);
    if !model_path.exists() {
        reset_to_idle(&a);
        show_error("Download a speech model in Setup");
        tray::refresh();
        return Err("no model".into());
    }
    // Make sure the model is (still) loaded; this is quick when it is.
    let sc = a.sidecar.clone();
    let mp = model_path.to_string_lossy().to_string();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = sc.load(&mp).await {
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
    pill::set(PillState::recording(&s.hotkeys.cancel.to_string()));
    tray::refresh();
    tauri::async_runtime::spawn(run_session(a.clone(), session, rec, audio_rx, ctl_rx, media_token));
    Ok(())
}

struct ChunkResult {
    index: usize,
    text: String,
    ms: u64,
    error: Option<String>,
}

async fn transcribe_worker(
    a: Arc<App>,
    mut rx: mpsc::UnboundedReceiver<Chunk>,
    language: String,
    prompt: String,
) -> Vec<ChunkResult> {
    let mut out = Vec::new();
    while let Some(chunk) = rx.recv().await {
        if !chunk.has_speech {
            continue;
        }
        let ctx = audio_ctx_for(chunk.samples.len());
        let timeout = Duration::from_millis(pipeline::CHUNK_TRANSCRIBE_TIMEOUT_MS);
        match a
            .sidecar
            .transcribe(&chunk.samples, &language, &prompt, ctx, timeout)
            .await
        {
            Ok((text, ms)) => out.push(ChunkResult {
                index: chunk.index,
                text,
                ms,
                error: None,
            }),
            Err(e) => out.push(ChunkResult {
                index: chunk.index,
                text: String::new(),
                ms: 0,
                error: Some(e),
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
) {
    let s = a.settings();
    let started = Instant::now();
    let mut chunker = Chunker::new(ChunkerConfig::default());
    // Whisper initial prompt: dictionary terms.
    let terms: Vec<String> = a.dict.lock().visible().iter().map(|e| e.term.clone()).collect();
    let term_refs: Vec<&str> = terms.iter().map(|t| t.as_str()).collect();
    let prompt = initial_prompt(&term_refs);
    let language = models::effective_language(s.model, &s.language);
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
    diag.model = Some(models::info(s.model).name.to_string());
    for r in &results {
        diag.push_stage(format!("Whisper chunk {}", r.index + 1), r.ms);
    }
    diag.push_stage("Whisper after stop", tail_ms);
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

    // Claude cleanup (or raw text if cleanup is off).
    let cleanup = if s.cleanup_enabled {
        diag.thinking = Some(false);
        let key = a.api_key.read().clone().unwrap_or_default();
        let entries_owned: Vec<_> = a.dict.lock().matching(&raw).into_iter().cloned().collect();
        let entries: Vec<&_> = entries_owned.iter().collect();
        let r = a
            .claude
            .cleanup(
                &key,
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
    match pipeline::after_cleanup(&raw, cleanup) {
        Final::Discard => {
            pill::set(PillState::Hidden);
        }
        Final::Insert { text, warning } => {
            let t = Instant::now();
            let outcome = tauri::async_runtime::spawn_blocking(move || insert_text(&mut SystemInsert, &text))
                .await
                .unwrap_or_else(|e| InsertOutcome::Failed(e.to_string()));
            diag.push_stage("Insert", t.elapsed().as_millis() as u64);
            report_insert(&mut diag, &outcome);
            match (outcome, warning) {
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

fn report_insert(diag: &mut DiagEntry, outcome: &InsertOutcome) {
    diag.insert_method = Some(match outcome {
        InsertOutcome::Pasted { restored: true } => "Clipboard paste (clipboard restored)".into(),
        InsertOutcome::Pasted { restored: false } => "Clipboard paste".into(),
        InsertOutcome::CopiedOnly {
            reason: PasteError::Blocked,
        } => "Copied only (paste blocked)".into(),
        InsertOutcome::CopiedOnly { reason } => format!("Copied only ({reason:?})"),
        InsertOutcome::Failed(e) => format!("Failed: {e}"),
    });
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
    tauri::async_runtime::spawn_blocking(move || insert_text(&mut SystemInsert, &text))
        .await
        .unwrap_or_else(|e| InsertOutcome::Failed(e.to_string()))
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
            let r = a
                .claude
                .cleanup(
                    &key,
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
                Ok(c) if !c.text.trim().is_empty() => {
                    diag.push_stage("Claude first byte", c.first_byte_ms);
                    diag.push_stage("Claude total", c.total_ms);
                    let cleaned = c.text.clone();
                    let t = Instant::now();
                    let outcome = paste(cleaned.clone()).await;
                    diag.push_stage("Insert", t.elapsed().as_millis() as u64);
                    report_insert(&mut diag, &outcome);
                    match outcome {
                        InsertOutcome::Pasted { .. } => {
                            a.undo.lock().record(&original, &cleaned);
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
