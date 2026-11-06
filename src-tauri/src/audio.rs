//! Microphone capture with cpal, converted to 16 kHz mono, and the soft
//! start/stop chime. cpal streams aren't Send, so each lives on its own thread.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;
use wirl_core::audio::Resampler;

pub struct Recording {
    stop_tx: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl Recording {
    /// Stops capture. Samples captured so far have already been sent.
    pub fn stop(mut self) {
        let _ = self.stop_tx.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(());
    }
}

pub fn has_input_device() -> bool {
    cpal::default_host().default_input_device().is_some()
}

fn build_stream(tx: tokio::sync::mpsc::UnboundedSender<Vec<f32>>) -> Result<Stream, String> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or("No microphone found")?;
    let cfg = device
        .default_input_config()
        .map_err(|e| format!("Microphone unavailable: {e}"))?;
    let channels = cfg.channels();
    let rate = cfg.sample_rate().0;
    let fmt = cfg.sample_format();
    let config: cpal::StreamConfig = cfg.into();
    let err_fn = |e| log::error!("microphone stream error: {e}");
    let mut rs = Resampler::new(rate, channels);
    let stream = match fmt {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _| {
                let _ = tx.send(rs.process(data));
            },
            err_fn,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _| {
                let f: Vec<f32> = data.iter().map(|s| *s as f32 / i16::MAX as f32).collect();
                let _ = tx.send(rs.process(&f));
            },
            err_fn,
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            &config,
            move |data: &[u16], _| {
                let f: Vec<f32> = data.iter().map(|s| (*s as f32 - 32768.0) / 32768.0).collect();
                let _ = tx.send(rs.process(&f));
            },
            err_fn,
            None,
        ),
        SampleFormat::I32 => device.build_input_stream(
            &config,
            move |data: &[i32], _| {
                let f: Vec<f32> = data.iter().map(|s| *s as f32 / i32::MAX as f32).collect();
                let _ = tx.send(rs.process(&f));
            },
            err_fn,
            None,
        ),
        other => return Err(format!("Unsupported microphone format {other:?}")),
    }
    .map_err(|e| format!("Couldn't open the microphone: {e}"))?;
    stream
        .play()
        .map_err(|e| format!("Couldn't start the microphone: {e}"))?;
    Ok(stream)
}

/// Starts capturing. Returns once the microphone is actually running.
pub fn start(tx: tokio::sync::mpsc::UnboundedSender<Vec<f32>>) -> Result<Recording, String> {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let thread = std::thread::Builder::new()
        .name("wirl-mic".into())
        .spawn(move || match build_stream(tx) {
            Ok(stream) => {
                let _ = ready_tx.send(Ok(()));
                let _ = stop_rx.recv();
                drop(stream);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e));
            }
        })
        .map_err(|e| e.to_string())?;
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(Recording {
            stop_tx,
            thread: Some(thread),
        }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("The microphone didn't respond".into()),
    }
}

/// A soft two-note chime: rising for start, falling for stop.
pub fn play_chime(start: bool) {
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let host = cpal::default_host();
            let dev = host.default_output_device().ok_or("no output")?;
            let cfg = dev.default_output_config().map_err(|e| e.to_string())?;
            let rate = cfg.sample_rate().0 as f32;
            let channels = cfg.channels() as usize;
            let notes: [f32; 2] = if start { [660.0, 880.0] } else { [880.0, 660.0] };
            let note_len = (rate * 0.085) as usize;
            let total = note_len * 2;
            let samples: Vec<f32> = (0..total)
                .map(|i| {
                    let n = i / note_len;
                    let t = (i % note_len) as f32 / rate;
                    let env_pos = (i % note_len) as f32 / note_len as f32;
                    // Soft attack and release.
                    let env = (env_pos * 12.0).min(1.0) * (1.0 - env_pos).powf(1.6);
                    0.12 * env * (std::f32::consts::TAU * notes[n] * t).sin()
                })
                .collect();
            let mut pos = 0usize;
            let config: cpal::StreamConfig = cfg.clone().into();
            let stream = match cfg.sample_format() {
                SampleFormat::F32 => dev.build_output_stream(
                    &config,
                    move |out: &mut [f32], _| {
                        for frame in out.chunks_mut(channels) {
                            let v = samples.get(pos).copied().unwrap_or(0.0);
                            pos += 1;
                            frame.iter_mut().for_each(|s| *s = v);
                        }
                    },
                    |e| log::warn!("chime: {e}"),
                    None,
                ),
                SampleFormat::I16 => dev.build_output_stream(
                    &config,
                    move |out: &mut [i16], _| {
                        for frame in out.chunks_mut(channels) {
                            let v = samples.get(pos).copied().unwrap_or(0.0);
                            pos += 1;
                            frame.iter_mut().for_each(|s| *s = (v * i16::MAX as f32) as i16);
                        }
                    },
                    |e| log::warn!("chime: {e}"),
                    None,
                ),
                other => return Err(format!("unsupported output format {other:?}")),
            }
            .map_err(|e| e.to_string())?;
            stream.play().map_err(|e| e.to_string())?;
            std::thread::sleep(Duration::from_millis(260));
            Ok(())
        })();
        if let Err(e) = result {
            log::warn!("chime skipped: {e}");
        }
    });
}
