//! Wirl's Whisper sidecar. Runs whisper.cpp in its own process so a crash or
//! out-of-memory here never takes down the app; the app restarts it.
//!
//! Protocol: one JSON message per line (see wirl_core::ipc). stdout carries
//! only protocol messages; whisper.cpp logs go to stderr, which the app
//! keeps a tail of for diagnostics.

use std::io::{BufRead, Write};
use std::time::Instant;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};
use wirl_core::audio::pad_to_min;
use wirl_core::ipc::{decode_pcm, Request, Response};

struct Loaded {
    // The state borrows nothing from the context in whisper-rs 0.16 (it keeps
    // an Arc), but keep the context alive alongside it regardless.
    _ctx: WhisperContext,
    state: WhisperState,
}

fn send(out: &mut impl Write, r: &Response) {
    let line = serde_json::to_string(r).expect("serialize response");
    // If the app has gone away there is nothing left to do.
    if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
        std::process::exit(0);
    }
}

fn threads() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8) as i32
}

fn load(path: &str, use_gpu: bool) -> Result<Loaded, String> {
    let mut p = WhisperContextParameters::default();
    p.use_gpu(use_gpu && cfg!(feature = "vulkan"));
    let ctx = WhisperContext::new_with_params(path, p).map_err(|e| format!("load model: {e}"))?;
    let state = ctx.create_state().map_err(|e| format!("create state: {e}"))?;
    Ok(Loaded { _ctx: ctx, state })
}

fn transcribe(l: &mut Loaded, pcm: &str, language: &str, prompt: &str, audio_ctx: i32) -> Result<String, String> {
    let mut samples = decode_pcm(pcm)?;
    if samples.is_empty() {
        return Ok(String::new());
    }
    pad_to_min(&mut samples);
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(threads());
    params.set_language(Some(language));
    if !prompt.is_empty() {
        params.set_initial_prompt(prompt);
    }
    params.set_audio_ctx(audio_ctx);
    params.set_no_context(true);
    params.set_no_timestamps(true);
    params.set_suppress_blank(true);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    l.state.full(params, &samples).map_err(|e| format!("transcribe: {e}"))?;
    let mut text = String::new();
    for seg in l.state.as_iter() {
        if let Ok(s) = seg.to_str_lossy() {
            text.push_str(&s);
        }
    }
    Ok(text.trim().to_string())
}

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    let mut loaded: Option<Loaded> = None;
    send(
        &mut out,
        &Response::Ready {
            version: env!("CARGO_PKG_VERSION").into(),
        },
    );

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                send(
                    &mut out,
                    &Response::Error {
                        id: None,
                        message: format!("bad request: {e}"),
                    },
                );
                continue;
            }
        };
        match req {
            Request::Load {
                model_path, use_gpu, ..
            } => {
                let t = Instant::now();
                loaded = None; // free the old model first
                match load(&model_path, use_gpu) {
                    Ok(l) => {
                        loaded = Some(l);
                        send(
                            &mut out,
                            &Response::Loaded {
                                model_path,
                                gpu: use_gpu && cfg!(feature = "vulkan"),
                                ms: t.elapsed().as_millis() as u64,
                            },
                        );
                    }
                    Err(message) => send(&mut out, &Response::Error { id: None, message }),
                }
            }
            Request::Transcribe {
                id,
                pcm16_b64,
                language,
                initial_prompt,
                audio_ctx,
            } => {
                let t = Instant::now();
                let Some(l) = loaded.as_mut() else {
                    send(
                        &mut out,
                        &Response::Error {
                            id: Some(id),
                            message: "no model loaded".into(),
                        },
                    );
                    continue;
                };
                match transcribe(l, &pcm16_b64, &language, &initial_prompt, audio_ctx) {
                    Ok(text) => send(
                        &mut out,
                        &Response::Result {
                            id,
                            text,
                            ms: t.elapsed().as_millis() as u64,
                        },
                    ),
                    Err(message) => send(&mut out, &Response::Error { id: Some(id), message }),
                }
            }
            Request::Ping { id } => send(&mut out, &Response::Pong { id }),
            Request::Shutdown => break,
        }
    }
}
