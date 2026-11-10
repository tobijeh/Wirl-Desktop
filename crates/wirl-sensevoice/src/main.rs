//! Wirl's SenseVoice sidecar (the Cantonese (粵語) and Mandarin (普通话) models). Runs
//! sherpa-onnx in its own process so a crash, hang or out-of-memory here
//! never takes down the app; the app times out, restarts it and reloads.
//!
//! Same protocol as the Whisper sidecar (wirl_core::ipc): one JSON message
//! per line, stdout carries only protocol messages, logs go to stderr.
//!
//! `Load.model_path` is the model folder (model.int8.onnx + tokens.txt).
//! `Load.language` and `Transcribe.language` are SenseVoice's language
//! ("yue" for Cantonese, "zh" for Mandarin);
//! the initial prompt and audio_ctx are Whisper-only and ignored here.
//!
//! Manual check on a WAV file (16-bit mono):
//!   wirl-sensevoice --model <folder> --language yue <file.wav>...

use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig};
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::Instant;
use wirl_core::audio::SAMPLE_RATE;
use wirl_core::filter::sensevoice_case;
use wirl_core::ipc::{decode_pcm, Request, Response};

const MODEL_FILE: &str = "model.int8.onnx";
const TOKENS_FILE: &str = "tokens.txt";

struct Loaded {
    dir: String,
    language: String,
    recognizer: OfflineRecognizer,
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
        .clamp(1, 4) as i32
}

fn create(dir: &str, language: &str) -> Result<Loaded, String> {
    create_with(dir, language, true)
}

fn create_with(dir: &str, language: &str, use_itn: bool) -> Result<Loaded, String> {
    let model = Path::new(dir).join(MODEL_FILE);
    let tokens = Path::new(dir).join(TOKENS_FILE);
    for f in [&model, &tokens] {
        if !f.is_file() {
            return Err(format!("missing {}", f.display()));
        }
    }
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
        model: Some(model.to_string_lossy().into_owned()),
        language: Some(language.to_string()),
        // Inverse text normalisation: punctuation and digits ("3點" not "三點").
        use_itn,
    };
    config.model_config.tokens = Some(tokens.to_string_lossy().into_owned());
    config.model_config.num_threads = threads();
    config.model_config.provider = Some("cpu".into());
    config.model_config.debug = false;
    let recognizer =
        OfflineRecognizer::create(&config).ok_or("load model: sherpa-onnx couldn't create the recognizer")?;
    Ok(Loaded {
        dir: dir.to_string(),
        language: language.to_string(),
        recognizer,
    })
}

fn recognize(l: &Loaded, samples: &[f32]) -> String {
    if samples.is_empty() {
        return String::new();
    }
    let stream = l.recognizer.create_stream();
    stream.accept_waveform(SAMPLE_RATE as i32, samples);
    l.recognizer.decode(&stream);
    stream
        .get_result()
        .map(|r| sensevoice_case(r.text.trim()))
        .unwrap_or_default()
}

/// The language the model is loaded with until a request asks for another.
const DEFAULT_LANGUAGE: &str = "yue";

fn cli(args: &[String]) -> Result<(), String> {
    let mut model = None;
    let mut language = DEFAULT_LANGUAGE.to_string();
    let mut use_itn = true;
    let mut files = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--model" => model = it.next().cloned(),
            "--no-itn" => use_itn = false,
            "--language" => language = it.next().cloned().ok_or("--language needs a value")?,
            f => files.push(f.to_string()),
        }
    }
    let model = model.ok_or("--model <folder> is required")?;
    let l = create_with(&model, &language, use_itn)?;
    for f in files {
        let wave = sherpa_onnx::Wave::read(&f).ok_or_else(|| format!("can't read {f}"))?;
        if wave.sample_rate() != SAMPLE_RATE as i32 {
            return Err(format!("{f}: expected {SAMPLE_RATE} Hz, got {}", wave.sample_rate()));
        }
        let t = Instant::now();
        let text = recognize(&l, wave.samples());
        println!("{f}\t{} ms\t{text}", t.elapsed().as_millis());
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        if let Err(e) = cli(&args) {
            eprintln!("{e}");
            std::process::exit(2);
        }
        return;
    }

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
                model_path, language, ..
            } => {
                let t = Instant::now();
                let language = if !language.is_empty() {
                    language
                } else {
                    loaded
                        .as_ref()
                        .map(|l| l.language.clone())
                        .unwrap_or_else(|| DEFAULT_LANGUAGE.into())
                };
                loaded = None; // free the old model first
                match create(&model_path, &language) {
                    Ok(l) => {
                        loaded = Some(l);
                        send(
                            &mut out,
                            &Response::Loaded {
                                model_path,
                                gpu: false,
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
                ..
            } => {
                let t = Instant::now();
                let Some(l) = loaded.as_ref() else {
                    send(
                        &mut out,
                        &Response::Error {
                            id: Some(id),
                            message: "no model loaded".into(),
                        },
                    );
                    continue;
                };
                // The language is part of the recognizer's config.
                if !language.is_empty() && language != l.language {
                    let dir = l.dir.clone();
                    loaded = None;
                    match create(&dir, &language) {
                        Ok(nl) => loaded = Some(nl),
                        Err(message) => {
                            send(&mut out, &Response::Error { id: Some(id), message });
                            continue;
                        }
                    }
                }
                let l = loaded.as_ref().expect("loaded above");
                match decode_pcm(&pcm16_b64) {
                    Ok(samples) => send(
                        &mut out,
                        &Response::Result {
                            id,
                            text: recognize(l, &samples),
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
