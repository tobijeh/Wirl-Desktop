//! Audio helpers: resampling to 16 kHz mono, voice-activity chunking and
//! whisper `audio_ctx` sizing.

pub const SAMPLE_RATE: usize = 16_000;
/// 30 ms analysis frames.
pub const FRAME: usize = 480;
const FRAME_MS: usize = 30;

/// Streaming down-mixer and resampler to 16 kHz mono. Uses an averaging
/// (box) filter when downsampling, which is enough to stop aliasing for
/// speech recognition.
pub struct Resampler {
    in_rate: f64,
    channels: usize,
    step: f64,
    pos: f64,
    mono: Vec<f32>,
}

impl Resampler {
    pub fn new(in_rate: u32, channels: u16) -> Self {
        let in_rate = in_rate.max(1) as f64;
        Resampler {
            in_rate,
            channels: channels.max(1) as usize,
            step: in_rate / SAMPLE_RATE as f64,
            pos: 0.0,
            mono: Vec::new(),
        }
    }

    pub fn input_rate(&self) -> f64 {
        self.in_rate
    }

    /// Feed interleaved samples, get 16 kHz mono samples out.
    pub fn process(&mut self, interleaved: &[f32]) -> Vec<f32> {
        for frame in interleaved.chunks(self.channels) {
            let s: f32 = frame.iter().sum::<f32>() / frame.len() as f32;
            self.mono.push(s);
        }
        let mut out = Vec::with_capacity((self.mono.len() as f64 / self.step) as usize + 1);
        let half = (self.step / 2.0).max(0.5);
        loop {
            let hi = self.pos + half;
            if hi.ceil() as usize >= self.mono.len() {
                break;
            }
            let lo = (self.pos - half).max(0.0);
            let (a, b) = (lo.floor() as usize, hi.ceil() as usize);
            if self.step <= 1.0 {
                // Upsampling: linear interpolation.
                let i = self.pos.floor() as usize;
                let f = (self.pos - i as f64) as f32;
                let x0 = self.mono[i];
                let x1 = self.mono[(i + 1).min(self.mono.len() - 1)];
                out.push(x0 + (x1 - x0) * f);
            } else {
                let slice = &self.mono[a..b.max(a + 1)];
                out.push(slice.iter().sum::<f32>() / slice.len() as f32);
            }
            self.pos += self.step;
        }
        // Drop consumed input, keep a little history.
        let keep_from = ((self.pos - half).floor().max(0.0)) as usize;
        if keep_from > 0 {
            self.mono.drain(..keep_from);
            self.pos -= keep_from as f64;
        }
        out
    }
}

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// Whisper `audio_ctx` for a chunk: 50 encoder positions per second of audio,
/// plus headroom, rounded up to 64, never below 384 or above 1500 (30 s).
pub fn audio_ctx_for(samples: usize) -> i32 {
    let secs = samples as f64 / SAMPLE_RATE as f64;
    let ctx = (secs * 50.0).ceil() as i32 + 64;
    let ctx = ((ctx + 63) / 64) * 64;
    ctx.clamp(384, 1500)
}

/// whisper.cpp rejects input under one second; pad with silence.
pub fn pad_to_min(samples: &mut Vec<f32>) {
    let min = SAMPLE_RATE + SAMPLE_RATE / 10;
    if samples.len() < min {
        samples.resize(min, 0.0);
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ChunkerConfig {
    /// Look for a pause once a chunk is at least this long.
    pub min_chunk_ms: usize,
    /// Force a cut (at the quietest point) by this length.
    pub max_chunk_ms: usize,
    /// Silence that counts as a natural pause.
    pub pause_ms: usize,
}

impl Default for ChunkerConfig {
    fn default() -> Self {
        ChunkerConfig {
            min_chunk_ms: 5_000,
            max_chunk_ms: 10_000,
            pause_ms: 300,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub index: usize,
    pub samples: Vec<f32>,
    pub has_speech: bool,
    pub is_final: bool,
}

impl Chunk {
    pub fn duration_ms(&self) -> usize {
        self.samples.len() * 1000 / SAMPLE_RATE
    }
}

/// Splits a live 16 kHz stream into chunks at natural pauses so most of a
/// dictation is transcribed while the user is still talking.
/// Frames used to measure the room when the microphone opens (240 ms).
const WARMUP_FRAMES: usize = 8;

/// Quiet voice counts as speech once it lasts this many frames in a row
/// (150 ms): longer than a click or a knock, shorter than a word.
const MIN_VOICE_RUN: usize = 5;

/// Normal speech anywhere, or quiet voice that lasts (a soft speaker or a
/// microphone set low).
fn contains_speech(speech: &[bool], voice: &[bool]) -> bool {
    if speech.iter().any(|s| *s) {
        return true;
    }
    let mut run = 0;
    for v in voice {
        run = if *v { run + 1 } else { 0 };
        if run >= MIN_VOICE_RUN {
            return true;
        }
    }
    false
}

pub struct Chunker {
    cfg: ChunkerConfig,
    buf: Vec<f32>,
    energies: Vec<f32>,
    speech: Vec<bool>,
    /// Quiet voice (trailing-off words) as well as normal speech. Used to
    /// find pauses and the end of speech, so quiet last words aren't cut.
    voice: Vec<bool>,
    noise_floor: f32,
    /// Background level, tracked from the quietest frames.
    floor: f32,
    /// Room level measured right after the microphone opens.
    warmup_min: f32,
    warmup_frames: usize,
    next_index: usize,
    total_samples: usize,
    any_speech: bool,
    last_level: f32,
}

impl Chunker {
    pub fn new(cfg: ChunkerConfig) -> Self {
        Chunker {
            cfg,
            buf: Vec::new(),
            energies: Vec::new(),
            speech: Vec::new(),
            voice: Vec::new(),
            noise_floor: 0.003,
            floor: 0.002,
            warmup_min: f32::MAX,
            warmup_frames: 0,
            next_index: 0,
            total_samples: 0,
            any_speech: false,
            last_level: 0.0,
        }
    }

    pub fn total_ms(&self) -> usize {
        self.total_samples * 1000 / SAMPLE_RATE
    }

    pub fn any_speech(&self) -> bool {
        self.any_speech
    }

    /// Latest frame RMS, for the pill's level wave.
    pub fn level(&self) -> f32 {
        self.last_level
    }

    fn is_speech(&mut self, e: f32) -> bool {
        let threshold = (self.noise_floor * 3.0).max(0.012);
        let speech = e > threshold;
        if !speech {
            self.noise_floor = self.noise_floor * 0.95 + e * 0.05;
        } else if e < self.noise_floor {
            self.noise_floor = e;
        }
        speech
    }

    /// Anything clearly above the room's background counts as voice, even
    /// when it's well below the speech threshold. The floor starts at a
    /// quiet room's level and follows the quietest frames: it drops at once,
    /// and outside speech rises slowly (about 18% a second), so a second or
    /// two of quiet talking can't raise it much.
    fn is_voice(&mut self, e: f32, speech: bool) -> bool {
        // The first ~240 ms (before anyone talks after the chime) set the
        // floor to the room's level, so a fan doesn't count as voice while
        // the slow rise catches up. Digital silence while the mic starts is
        // skipped.
        if self.warmup_frames < WARMUP_FRAMES && (e > 0.0002 || self.warmup_frames > 0) {
            self.warmup_frames += 1;
            if !speech {
                self.warmup_min = self.warmup_min.min(e);
            }
            if self.warmup_frames == WARMUP_FRAMES && self.warmup_min != f32::MAX && self.warmup_min > self.floor {
                self.floor = self.warmup_min;
            }
        }
        if e < self.floor {
            self.floor = e.max(0.0003);
        } else if !speech {
            self.floor = (self.floor * 1.005).min(e);
        }
        // Until the room is measured, only normal speech counts (the audio
        // itself is kept either way).
        self.warmup_frames >= WARMUP_FRAMES && e > (self.floor * 3.0).max(0.0025)
    }

    pub fn push(&mut self, samples: &[f32]) -> Vec<Chunk> {
        self.total_samples += samples.len();
        self.buf.extend_from_slice(samples);
        let mut out = Vec::new();
        while self.energies.len() * FRAME + FRAME <= self.buf.len() {
            let start = self.energies.len() * FRAME;
            let e = rms(&self.buf[start..start + FRAME]);
            self.last_level = e;
            let sp = self.is_speech(e);
            let voice = self.is_voice(e, sp) || sp;
            self.energies.push(e);
            self.speech.push(sp);
            self.voice.push(voice);
            if sp || self.voice.iter().rev().take_while(|v| **v).count() >= MIN_VOICE_RUN {
                self.any_speech = true;
            }
            if let Some(c) = self.maybe_cut() {
                out.push(c);
            }
        }
        out
    }

    fn silence_run(&self) -> usize {
        self.voice.iter().rev().take_while(|s| !**s).count()
    }

    fn maybe_cut(&mut self) -> Option<Chunk> {
        let frames = self.energies.len();
        let has_speech = contains_speech(&self.speech, &self.voice);
        // Drop leading silence beyond 300 ms so chunks start near speech.
        if !has_speech && frames * FRAME_MS > 1_000 {
            let drop = frames - 300 / FRAME_MS;
            self.drain_frames(drop);
            return None;
        }
        let dur = frames * FRAME_MS;
        let run = self.silence_run();
        if has_speech && dur >= self.cfg.min_chunk_ms && run * FRAME_MS >= self.cfg.pause_ms {
            return Some(self.cut_at(frames - run / 2));
        }
        if dur >= self.cfg.max_chunk_ms {
            // Cut at the quietest point (5-frame smoothed) after min length.
            let min_f = self.cfg.min_chunk_ms / FRAME_MS;
            let mut best = frames;
            let mut best_e = f32::MAX;
            for i in min_f.max(2)..frames.saturating_sub(2) {
                let e: f32 = self.energies[i - 2..=i + 2].iter().sum();
                if e < best_e {
                    best_e = e;
                    best = i;
                }
            }
            return Some(self.cut_at(best));
        }
        None
    }

    fn drain_frames(&mut self, n: usize) {
        self.buf.drain(..n * FRAME);
        self.energies.drain(..n);
        self.speech.drain(..n);
        self.voice.drain(..n);
    }

    fn cut_at(&mut self, frame: usize) -> Chunk {
        let samples: Vec<f32> = self.buf[..frame * FRAME].to_vec();
        let has_speech = contains_speech(&self.speech[..frame], &self.voice[..frame]);
        self.drain_frames(frame);
        let c = Chunk {
            index: self.next_index,
            samples,
            has_speech,
            is_final: false,
        };
        self.next_index += 1;
        c
    }

    /// Ends the stream and returns the short final chunk (silence more than
    /// 400 ms after the last voice trimmed; quiet trailing words are voice
    /// and stay). Called the instant recording stops.
    pub fn finish(&mut self) -> Option<Chunk> {
        let frames = self.energies.len();
        let run = self.silence_run();
        let keep_silence = 400 / FRAME_MS;
        let mut end = self.buf.len();
        let tail_speech = contains_speech(&self.speech, &self.voice);
        if run > keep_silence && tail_speech {
            end = (frames - run + keep_silence) * FRAME;
        }
        if end == 0 {
            return None;
        }
        let samples = self.buf[..end].to_vec();
        self.buf.clear();
        self.energies.clear();
        self.speech.clear();
        self.voice.clear();
        let c = Chunk {
            index: self.next_index,
            samples,
            has_speech: tail_speech,
            is_final: true,
        };
        self.next_index += 1;
        Some(c)
    }
}

/// Joins chunk transcripts in order.
pub fn assemble(parts: &[(usize, String)]) -> String {
    let mut v: Vec<&(usize, String)> = parts.iter().collect();
    v.sort_by_key(|p| p.0);
    let mut out = String::new();
    for t in v.iter().map(|p| p.1.trim()).filter(|t| !t.is_empty()) {
        // No space between two Chinese pieces; English keeps its spaces.
        let tight = out.chars().next_back().is_some_and(is_cjk) && t.chars().next().is_some_and(is_cjk);
        if !out.is_empty() && !tight {
            out.push(' ');
        }
        out.push_str(t);
    }
    out
}

/// Characters written without spaces around them: Chinese characters, kana
/// and full-width punctuation.
fn is_cjk(c: char) -> bool {
    crate::dictionary::is_han(c) || matches!(c, '\u{3000}'..='\u{30FF}' | '\u{FF00}'..='\u{FFEF}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(ms: usize, amp: f32) -> Vec<f32> {
        let n = ms * SAMPLE_RATE / 1000;
        (0..n).map(|i| amp * ((i as f32) * 0.07).sin()).collect()
    }
    fn silence(ms: usize) -> Vec<f32> {
        vec![0.0; ms * SAMPLE_RATE / 1000]
    }

    #[test]
    fn assemble_joins_in_order_without_spaces_in_chinese() {
        let parts = vec![(1, " world ".to_string()), (0, "hello".into()), (2, "".into())];
        assert_eq!(assemble(&parts), "hello world");
        let zh = vec![
            (1, "之后要 SEND 个 EMAIL".to_string()),
            (0, "我今日有个 MEETING".into()),
            (2, "OK".into()),
        ];
        assert_eq!(assemble(&zh), "我今日有个 MEETING 之后要 SEND 个 EMAIL OK");
        let zh3 = vec![(0, "我今日".to_string()), (1, "有个 MEETING".into())];
        assert_eq!(assemble(&zh3), "我今日有个 MEETING");
        let zh2 = vec![(0, "你好。".to_string()), (1, "我系".into())];
        assert_eq!(assemble(&zh2), "你好。我系");
    }

    #[test]
    fn audio_ctx_scales_with_length() {
        assert_eq!(audio_ctx_for(SAMPLE_RATE / 2), 384); // minimum
        assert_eq!(audio_ctx_for(SAMPLE_RATE * 8), 512); // 400 + 64 -> 464 -> 512
        assert_eq!(audio_ctx_for(SAMPLE_RATE * 10), 576);
        assert_eq!(audio_ctx_for(SAMPLE_RATE * 60), 1500); // capped
        assert!(audio_ctx_for(SAMPLE_RATE * 3) < audio_ctx_for(SAMPLE_RATE * 9));
    }

    #[test]
    fn cuts_at_pause_after_min_length() {
        let mut c = Chunker::new(ChunkerConfig::default());
        let mut chunks = c.push(&tone(6_000, 0.3));
        assert!(chunks.is_empty());
        chunks.extend(c.push(&silence(400)));
        assert_eq!(chunks.len(), 1);
        let first = &chunks[0];
        assert!(first.has_speech);
        assert!(
            first.duration_ms() >= 6_000 && first.duration_ms() < 6_400,
            "{}",
            first.duration_ms()
        );
        c.push(&tone(1_500, 0.3));
        let fin = c.finish().unwrap();
        assert!(fin.is_final);
        assert_eq!(fin.index, 1);
        assert!(fin.duration_ms() < 2_000);
    }

    #[test]
    fn no_cut_on_short_pauses_before_min() {
        let mut c = Chunker::new(ChunkerConfig::default());
        let mut n = 0;
        for _ in 0..3 {
            n += c.push(&tone(900, 0.3)).len();
            n += c.push(&silence(400)).len();
        }
        n += c.push(&tone(900, 0.3)).len();
        assert_eq!(n, 0); // 4.8 s: every pause came before 5 s of audio
    }

    #[test]
    fn forces_cut_by_max_length() {
        let mut c = Chunker::new(ChunkerConfig::default());
        let chunks = c.push(&tone(12_000, 0.3));
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].duration_ms() <= 10_000 && chunks[0].duration_ms() >= 5_000);
    }

    #[test]
    fn leading_silence_is_trimmed_and_flagged() {
        let mut c = Chunker::new(ChunkerConfig::default());
        c.push(&silence(5_000));
        assert!(!c.any_speech());
        let fin = c.finish().unwrap();
        assert!(!fin.has_speech);
        assert!(fin.duration_ms() <= 1_100);
    }

    /// A voice trailing off at the end of a sentence: louder speech, then the
    /// last words well below the speech threshold, then the stop key.
    #[test]
    fn quiet_last_words_are_kept_when_stopping_quickly() {
        for noise in [0.0, 0.002] {
            let mut c = Chunker::new(ChunkerConfig::default());
            let mut audio = tone(2_000, 0.3);
            audio.extend(tone(1_200, 0.015)); // rms ~0.011, under the 0.012 speech threshold
            audio.extend(silence(300));
            for (i, s) in audio.iter_mut().enumerate() {
                *s += noise * ((i as f32) * 1.3).sin();
            }
            assert!(c.push(&audio).is_empty());
            let fin = c.finish().unwrap();
            assert!(fin.has_speech);
            assert!(
                fin.duration_ms() >= 3_200,
                "noise {noise}: {} ms kept",
                fin.duration_ms()
            );
        }
    }

    /// A soft speaker or a microphone set low: every word is under the
    /// normal speech threshold. Before, all but the last 300 ms was dropped
    /// as "silence" and Whisper typed a fragment like "I".
    #[test]
    fn quiet_speaker_is_heard_from_the_start() {
        let mut c = Chunker::new(ChunkerConfig::default());
        let mut audio = silence(500);
        for _ in 0..4 {
            audio.extend(tone(700, 0.008)); // rms ~0.0057
            audio.extend(silence(150));
        }
        audio.extend(silence(300));
        assert!(c.push(&audio).is_empty());
        assert!(c.any_speech());
        let fin = c.finish().unwrap();
        assert!(fin.has_speech);
        assert!(fin.duration_ms() >= 3_300, "{} ms kept", fin.duration_ms());
    }

    /// Steady background noise (a fan) under the speech threshold is not
    /// speech, whatever its level.
    #[test]
    fn steady_background_noise_is_not_speech() {
        for amp in [0.003, 0.008, 0.014] {
            let mut c = Chunker::new(ChunkerConfig::default());
            let noise: Vec<f32> = (0..SAMPLE_RATE * 5).map(|i| amp * ((i as f32) * 1.7).sin()).collect();
            c.push(&noise);
            assert!(!c.any_speech(), "amp {amp}");
            assert!(!c.finish().unwrap().has_speech, "amp {amp}");
        }
    }

    #[test]
    fn pause_cut_does_not_split_quiet_words() {
        let mut c = Chunker::new(ChunkerConfig::default());
        let mut chunks = c.push(&tone(5_500, 0.3));
        chunks.extend(c.push(&tone(900, 0.015)));
        chunks.extend(c.push(&silence(700)));
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].duration_ms() >= 6_400, "{} ms", chunks[0].duration_ms());
    }

    #[test]
    fn final_chunk_trims_trailing_silence() {
        let mut c = Chunker::new(ChunkerConfig::default());
        c.push(&tone(2_000, 0.3));
        c.push(&silence(2_000));
        let fin = c.finish().unwrap();
        assert!(fin.duration_ms() < 2_500, "{}", fin.duration_ms());
    }

    #[test]
    fn assemble_orders_by_index() {
        let parts = vec![
            (1, " world".to_string()),
            (0, "hello ".to_string()),
            (2, "".to_string()),
        ];
        assert_eq!(assemble(&parts), "hello world");
    }

    #[test]
    fn resampler_48k_stereo_to_16k_mono() {
        let mut r = Resampler::new(48_000, 2);
        let input: Vec<f32> = (0..48_000 * 2).map(|i| if i % 2 == 0 { 0.5 } else { 0.1 }).collect();
        let out = r.process(&input);
        assert!((out.len() as i64 - 16_000).abs() < 4, "{}", out.len());
        assert!((out[100] - 0.3).abs() < 1e-4);
    }

    #[test]
    fn resampler_44k1() {
        let mut r = Resampler::new(44_100, 1);
        let mut total = 0;
        for _ in 0..10 {
            total += r.process(&vec![0.0; 4_410]).len();
        }
        assert!((total as i64 - 16_000).abs() < 4, "{total}");
    }

    #[test]
    fn pad_min() {
        let mut v = vec![0.1; 100];
        pad_to_min(&mut v);
        assert!(v.len() > SAMPLE_RATE);
    }
}
