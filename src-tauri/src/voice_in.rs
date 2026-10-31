//! Hands-free listening: "Hey Pyrosan, …" or the push-to-talk shortcut. The microphone audio
//! never leaves this computer: a local speech model (whisper.cpp) turns it into text, and only
//! that text goes on, exactly like a typed message.

use crate::launcher::normalize;
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
        Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};

pub const MODEL_FILE: &str = "models/ggml-small-q5_1.bin";
/// The last sentence heard, as a sound file in the app's data folder (for /micro).
pub const LAST_SENTENCE: &str = "derniere-phrase.wav";
const RATE: usize = 16_000;
const FRAME: usize = RATE / 50; // 20 ms
const START_FRAMES: usize = 4; // 80 ms of voice starts an utterance
const END_FRAMES: usize = 40; // 800 ms of quiet ends it
const PREROLL_FRAMES: usize = 15; // keep 300 ms from before it started
const MIN_VOICED_FRAMES: usize = 15; // ignore clicks and bumps under 300 ms
const MAX_FRAMES: usize = 15 * 50; // 15 s
/// Quietest 20 ms that can still count as voice (about -46 dB), for low-gain headset mics.
const MIN_VOICE: f32 = 0.005;
/// After "Hey Pyrosan" alone (or the shortcut), how long the pet waits for the request.
const FOLLOW_UP: Duration = Duration::from_secs(10);

/// Turns microphone audio at any rate into 16 kHz by averaging (which also smooths it).
pub struct Downsampler {
    step: f64,
    acc: f64,
    sum: f32,
    n: u32,
}

impl Downsampler {
    pub fn new(input_rate: u32) -> Self {
        Self { step: (input_rate as f64 / RATE as f64).max(1.0), acc: 0.0, sum: 0.0, n: 0 }
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        for &x in input {
            self.sum += x;
            self.n += 1;
            self.acc += 1.0;
            if self.acc >= self.step {
                out.push(self.sum / self.n as f32);
                self.acc -= self.step;
                self.sum = 0.0;
                self.n = 0;
            }
        }
    }
}

/// Finds where someone starts and stops talking, from loudness against the room's own noise.
pub struct Vad {
    floor: f32,
    partial: Vec<f32>,
    preroll: std::collections::VecDeque<Vec<f32>>,
    utterance: Vec<f32>,
    voiced_run: usize,
    voiced_total: usize,
    quiet_run: usize,
    talking: bool,
}

impl Default for Vad {
    fn default() -> Self {
        Self {
            floor: 0.003,
            partial: Vec::new(),
            preroll: Default::default(),
            utterance: Vec::new(),
            voiced_run: 0,
            voiced_total: 0,
            quiet_run: 0,
            talking: false,
        }
    }
}

impl Vad {
    pub fn reset(&mut self) {
        *self = Self { floor: self.floor, ..Default::default() };
    }

    pub fn talking(&self) -> bool {
        self.talking
    }

    /// Feeds 16 kHz audio; returns every utterance that just ended.
    pub fn push(&mut self, samples: &[f32]) -> Vec<Vec<f32>> {
        let mut done = Vec::new();
        self.partial.extend_from_slice(samples);
        while self.partial.len() >= FRAME {
            let frame: Vec<f32> = self.partial.drain(..FRAME).collect();
            let rms = (frame.iter().map(|x| x * x).sum::<f32>() / FRAME as f32).sqrt();
            let voiced = rms > (self.floor * 2.5).max(MIN_VOICE);
            if !self.talking {
                // The background level follows the room while nobody's talking.
                self.floor = (self.floor * 0.97 + rms * 0.03).clamp(0.001, 0.05);
                self.preroll.push_back(frame);
                if self.preroll.len() > PREROLL_FRAMES {
                    self.preroll.pop_front();
                }
                self.voiced_run = if voiced { self.voiced_run + 1 } else { 0 };
                if self.voiced_run >= START_FRAMES {
                    self.talking = true;
                    self.utterance = self.preroll.drain(..).flatten().collect();
                    self.voiced_total = self.voiced_run;
                    self.quiet_run = 0;
                }
                continue;
            }
            self.utterance.extend_from_slice(&frame);
            if voiced {
                self.quiet_run = 0;
                self.voiced_total += 1;
            } else {
                self.quiet_run += 1;
            }
            if self.quiet_run >= END_FRAMES || self.utterance.len() >= MAX_FRAMES * FRAME {
                let utterance = std::mem::take(&mut self.utterance);
                if self.voiced_total >= MIN_VOICED_FRAMES {
                    done.push(utterance);
                }
                self.talking = false;
                self.voiced_run = 0;
                self.voiced_total = 0;
            }
        }
        done
    }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for j in 0..b.len() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != b[j])).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// If `text` starts with the pet's name (maybe after "hey"), what comes after it.
/// Speech models mishear names, so "Et Pyro-san", "Pirosan" or "Yrosan" all count.
pub fn after_wake_word(text: &str, name: &str) -> Option<String> {
    let wanted = normalize(name).replace(' ', "");
    if wanted.is_empty() {
        return None;
    }
    let slack = match wanted.chars().count() {
        0..=4 => 0,
        5..=6 => 1,
        _ => 2,
    };
    let words: Vec<&str> = text.split_whitespace().collect();
    let plain: Vec<String> = words.iter().map(|w| normalize(w).replace(' ', "")).collect();
    let greetings = ["hey", "he", "hei", "et", "eh", "e", "ok", "okay", "salut", "dis"];
    for start in 0..=1.min(words.len()) {
        if start == 1 && !greetings.contains(&plain[0].as_str()) {
            break;
        }
        let mut joined = String::new();
        for (k, word) in plain.iter().enumerate().skip(start).take(3) {
            joined.push_str(word);
            if !joined.is_empty() && levenshtein(&joined, &wanted) <= slack {
                let rest = words[k + 1..].join(" ");
                return Some(rest.trim_start_matches(|c: char| c == ',' || c == '.' || c == '!' || c.is_whitespace()).to_string());
            }
        }
    }
    None
}

/// Drops what speech models invent on noise: "(signal)", "[Musique]", subtitle credits…
pub fn clean(text: &str) -> String {
    static NOISE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\([^)]*\)|\[[^\]]*\]|\*[^*]*\*").unwrap());
    let out = NOISE.replace_all(text, " ");
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let plain = normalize(&out);
    const INVENTED: [&str; 6] = ["sous titres", "sous titrage", "amara org", "merci d avoir regarde", "abonnez vous", "merci de votre attention"];
    if plain.is_empty() || INVENTED.iter().any(|s| plain.contains(s)) {
        return String::new();
    }
    out
}

/// Loudest 20 ms block in some 16 kHz audio.
fn vad_peak_value(pcm: &[f32]) -> f32 {
    pcm.chunks(FRAME).map(|f| (f.iter().map(|x| x * x).sum::<f32>() / f.len() as f32).sqrt()).fold(0.0, f32::max)
}

/// What the window shows: listening, thinking, idle (with a note), or a problem.
#[derive(Clone, Serialize)]
pub struct Listen {
    pub state: &'static str,
    pub text: String,
}

fn tell(app: &AppHandle, state: &'static str, text: impl Into<String>) {
    let _ = app.emit("listen", Listen { state, text: text.into() });
}

/// Shared between the listening thread and the rest of the app.
pub struct Ears {
    wake: AtomicBool,
    speaking: AtomicBool,
    /// When the pet started talking, so a lost "stopped talking" can't mute the mic forever.
    speaking_since: Mutex<Option<Instant>>,
    armed_until: Mutex<Option<Instant>>,
    /// What wakes the pet up: its name, or the words chosen with /settings wake.
    name: Mutex<Vec<String>>,
    /// App names from the Start menu, to help the speech model spell "lance League of Legends".
    vocabulary: Mutex<String>,
    /// What /micro reports: which mic, how loud, what was last understood, what went wrong.
    report: Mutex<Report>,
}

#[derive(Default)]
struct Report {
    model: Option<bool>,
    mic: String,
    /// Loudest 20 ms in the last half minute or so, and when it was last raised.
    peak: f32,
    peak_at: Option<Instant>,
    /// How loud the last sentence was, before the automatic boost.
    last_sentence_db: Option<f32>,
    heard: Vec<String>,
    problem: String,
}

fn db(level: f32) -> f32 {
    20.0 * level.max(1e-6).log10()
}

/// Brings a quiet sentence up to a level the speech model hears well (at most ×30).
pub fn louder(audio: &[f32]) -> Vec<f32> {
    let peak = audio.iter().fold(0f32, |m, x| m.max(x.abs()));
    let rms = (audio.iter().map(|x| x * x).sum::<f32>() / audio.len().max(1) as f32).sqrt();
    if peak < 1e-4 {
        return audio.to_vec();
    }
    let gain = (0.9 / peak).min(0.1 / rms.max(1e-6)).clamp(1.0, 30.0);
    audio.iter().map(|x| (x * gain).clamp(-1.0, 1.0)).collect()
}

impl Ears {
    /// A plain-French summary of the listening state, for /micro.
    pub fn describe(&self) -> String {
        let r = self.report.lock().unwrap();
        let wake = self.wake.load(Ordering::SeqCst);
        let model = match r.model {
            Some(true) => "chargé ✅",
            Some(false) => "introuvable ❌",
            None => "pas encore chargé",
        };
        let mic = if r.mic.is_empty() { "pas encore ouvert".to_string() } else { r.mic.clone() };
        let recent = r.peak_at.is_some_and(|t| t.elapsed() < Duration::from_secs(30));
        let level = if !recent || r.peak <= 0.0 {
            "rien de fort depuis 30 s".to_string()
        } else {
            let verdict = if r.peak < MIN_VOICE * 4.0 { " (faible : monte le micro dans Windows)" } else { " (ok)" };
            format!("{:.0} dB{verdict}", db(r.peak))
        };
        let level = match r.last_sentence_db {
            Some(d) => format!("{level}\ndernière phrase : {d:.0} dB (je l'amplifie au besoin)"),
            None => format!("{level}\ndernière phrase : aucune détectée"),
        };
        let heard = if r.heard.is_empty() { "rien".to_string() } else { r.heard.iter().map(|h| format!("« {h} »")).collect::<Vec<_>>().join(" · ") };
        let mut out = format!(
            "écoute « {} » : {}\nmodèle vocal : {model}\nmicro : {mic}\nvolume max récent : {level}\nderniers mots entendus : {heard}",
            self.name.lock().unwrap().join(" » ou « "),
            if wake { "oui" } else { "non" }
        );
        if !r.problem.is_empty() {
            out.push_str(&format!("\nproblème : {}", r.problem));
        }
        out
    }

    fn note_heard(&self, text: &str, raw: &Result<String, String>) {
        let mut r = self.report.lock().unwrap();
        r.heard.push(match raw {
            _ if !text.is_empty() => text.to_string(),
            Ok(raw) if raw.is_empty() => "… (rien reconnu)".into(),
            Ok(raw) => format!("… (brut : {raw})"),
            Err(e) => format!("… ({e})"),
        });
        if r.heard.len() > 3 {
            r.heard.remove(0);
        }
    }
}

impl Ears {
    pub fn new(wake: bool, words: Vec<String>) -> Self {
        Self {
            wake: AtomicBool::new(wake),
            speaking: AtomicBool::new(false),
            speaking_since: Mutex::new(None),
            armed_until: Mutex::new(None),
            name: Mutex::new(words),
            vocabulary: Mutex::new(String::new()),
            report: Mutex::new(Report::default()),
        }
    }

    pub fn set_wake(&self, on: bool) {
        self.wake.store(on, Ordering::SeqCst);
    }

    pub fn set_words(&self, words: Vec<String>) {
        *self.name.lock().unwrap() = words;
    }

    /// While the pet talks, the microphone would hear it: ignore it then.
    pub fn set_speaking(&self, on: bool) {
        self.speaking.store(on, Ordering::SeqCst);
        *self.speaking_since.lock().unwrap() = on.then(Instant::now);
    }

    fn is_speaking(&self) -> bool {
        // No answer is read for more than two minutes: past that, "stopped" was lost.
        if self.speaking_since.lock().unwrap().is_some_and(|t| t.elapsed() > Duration::from_secs(120)) {
            self.set_speaking(false);
        }
        self.speaking.load(Ordering::SeqCst)
    }

    /// The next thing said is a request (no "hey Pyrosan" needed).
    pub fn arm(&self) {
        *self.armed_until.lock().unwrap() = Some(Instant::now() + FOLLOW_UP);
    }

    pub fn cancel(&self) {
        *self.armed_until.lock().unwrap() = None;
    }

    fn armed(&self) -> Option<Instant> {
        *self.armed_until.lock().unwrap()
    }

    fn take_armed(&self) -> bool {
        self.armed_until.lock().unwrap().take().is_some_and(|until| Instant::now() <= until + Duration::from_secs(15))
    }
}

/// Where the speech model was installed, if it was.
pub fn model_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().resolve(MODEL_FILE, tauri::path::BaseDirectory::Resource).ok().filter(|p| p.is_file())
}

/// Half the cores, up to 8: fast enough to answer quickly, light enough for a game running.
fn threads() -> i32 {
    let cores = thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    (cores / 2).clamp(1, 8) as i32
}

/// The first wake word that starts `text`, and what comes after it.
fn after_any(text: &str, words: &[String]) -> Option<String> {
    words.iter().find_map(|w| after_wake_word(text, w))
}

/// Turns speech into text. `retry` is the second, more patient attempt after an empty result:
/// no app names in the prompt and, if the first guess is shaky, a few warmer guesses.
fn transcribe(ctx: &whisper_rs::WhisperContext, audio: &[f32], prompt: &str, retry: bool) -> Result<String, String> {
    use whisper_rs::{FullParams, SamplingStrategy};
    let mut state = ctx.create_state().map_err(|e| format!("modèle vocal : {e}"))?;
    let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    p.set_language(Some("fr"));
    p.set_n_threads(threads());
    p.set_print_progress(false);
    p.set_print_realtime(false);
    p.set_print_special(false);
    p.set_print_timestamps(false);
    p.set_no_context(true);
    p.set_single_segment(true);
    p.set_no_timestamps(true);
    p.set_suppress_blank(true);
    p.set_temperature_inc(if retry { 0.2 } else { 0.0 });
    if retry {
        // Don't drop a segment for looking like silence: we know someone spoke.
        p.set_no_speech_thold(1.0);
    }
    p.set_initial_prompt(prompt);
    // whisper.cpp wants at least a second of audio.
    let mut padded = audio.to_vec();
    padded.resize(padded.len().max(RATE * 6 / 5), 0.0);
    state.full(p, &padded).map_err(|e| format!("reconnaissance vocale : {e}"))?;
    Ok(state.as_iter().filter_map(|s| s.to_str_lossy().ok().map(|t| t.into_owned())).collect::<Vec<_>>().join(" ").trim().to_string())
}

/// Keeps the last sentence as a sound file, to check what the pet really heard.
fn save_wav(path: &std::path::Path, audio: &[f32]) {
    let mut bytes = Vec::with_capacity(44 + audio.len() * 2);
    let data_len = (audio.len() * 2) as u32;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&(RATE as u32).to_le_bytes());
    bytes.extend_from_slice(&(RATE as u32 * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for x in audio {
        bytes.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, bytes);
}

struct Mic {
    _stream: cpal::Stream,
    rx: Receiver<(Instant, Vec<f32>)>,
    down: Downsampler,
    name: String,
}

fn open_mic() -> Result<Mic, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let device = cpal::default_host().default_input_device().ok_or("je ne trouve pas de micro")?;
    let config = device.default_input_config().map_err(|e| format!("micro inutilisable ({e})"))?;
    let channels = config.channels() as usize;
    let rate = config.sample_rate().0;
    let (tx, rx) = mpsc::channel::<(Instant, Vec<f32>)>();
    let mono = move |samples: Vec<f32>| -> Vec<f32> { samples.chunks(channels).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect() };
    let fail = |e| eprintln!("microphone error: {e}");
    let stream_config: cpal::StreamConfig = config.clone().into();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |data: &[f32], _: &_| drop(tx.send((Instant::now(), mono(data.to_vec())))),
            fail,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &stream_config,
            move |data: &[i16], _: &_| drop(tx.send((Instant::now(), mono(data.iter().map(|&s| s as f32 / 32768.0).collect())))),
            fail,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            &stream_config,
            move |data: &[u16], _: &_| {
                drop(tx.send((Instant::now(), mono(data.iter().map(|&s| (s as f32 - 32768.0) / 32768.0).collect()))))
            },
            fail,
            None,
        ),
        other => return Err(format!("format de micro non géré ({other})")),
    }
    .map_err(|e| format!("micro inutilisable ({e})"))?;
    stream.play().map_err(|e| format!("micro inutilisable ({e})"))?;
    let name = device.name().unwrap_or_else(|_| "micro par défaut".into());
    Ok(Mic { _stream: stream, rx, down: Downsampler::new(rate), name: format!("{name} ({rate} Hz, {channels} canal/aux)") })
}

fn handle(app: &AppHandle, ears: &Ears, ctx: &whisper_rs::WhisperContext, audio: &[f32]) {
    let busy = app.state::<crate::AppState>().thinking.load(Ordering::SeqCst);
    let armed = ears.take_armed();
    // While it's already working on an answer, saying it again would only queue up repeats.
    if !armed && (busy || !ears.wake.load(Ordering::SeqCst)) {
        return;
    }
    let words = ears.name.lock().unwrap().clone();
    if armed {
        tell(app, "thinking", "");
    }
    let vocabulary: String = ears.vocabulary.lock().unwrap().chars().take(300).collect();
    let rms = (audio.iter().map(|x| x * x).sum::<f32>() / audio.len().max(1) as f32).sqrt();
    ears.report.lock().unwrap().last_sentence_db = Some(db(rms));
    let boosted = louder(audio);
    if let Ok(dir) = app.path().app_data_dir() {
        save_wav(&dir.join(LAST_SENTENCE), &boosted);
    }
    // Naming the pet and the user's apps up front helps the model spell them right.
    let simple = format!("Hey {}.", words.join(". Hey "));
    let with_apps = if vocabulary.is_empty() { simple.clone() } else { format!("{vocabulary}. {simple}") };
    let mut raw = transcribe(ctx, &boosted, &with_apps, false);
    if raw.as_ref().map_or(true, |t| clean(t).is_empty()) {
        raw = transcribe(ctx, &boosted, &simple, true);
    }
    let text = raw.as_ref().map(|t| clean(t)).unwrap_or_default();
    ears.note_heard(&text, &raw);
    if armed {
        let request = after_any(&text, &words).unwrap_or(text);
        if request.is_empty() {
            tell(app, "idle", "je n'ai rien entendu");
        } else {
            let _ = app.emit("heard", request);
        }
        return;
    }
    let Some(request) = after_any(&text, &words) else { return };
    if request.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
        // Just "Hey Pyrosan": the request comes next.
        ears.arm();
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
        }
        tell(app, "listening", "");
    } else {
        let _ = app.emit("heard", request);
    }
}

/// The listening thread: opens the microphone only while it's needed (wake word on, or
/// waiting for a request) and loads the speech model the first time.
pub fn start(app: AppHandle) {
    thread::spawn(move || {
        whisper_rs::install_logging_hooks();
        let ears = app.state::<Ears>();
        *ears.vocabulary.lock().unwrap() = crate::launcher::app_names(30).join(", ");
        let mut model: Option<whisper_rs::WhisperContext> = None;
        let mut mic: Option<Mic> = None;
        let mut vad = Vad::default();
        let mut complained = false;
        loop {
            let armed = ears.armed();
            if !ears.wake.load(Ordering::SeqCst) && armed.is_none() {
                mic = None;
                vad.reset();
                thread::sleep(Duration::from_millis(250));
                continue;
            }
            if model.is_none() {
                let loaded = model_path(&app).ok_or("modèle vocal introuvable (réinstalle Pyrosan)".to_string()).and_then(|path| {
                    whisper_rs::WhisperContext::new_with_params(&path, whisper_rs::WhisperContextParameters::default())
                        .map_err(|e| format!("modèle vocal illisible ({e})"))
                });
                match loaded {
                    Ok(ctx) => {
                        ears.report.lock().unwrap().model = Some(true);
                        model = Some(ctx)
                    }
                    Err(e) => {
                        let mut r = ears.report.lock().unwrap();
                        r.model = Some(false);
                        r.problem = e.clone();
                        drop(r);
                        if armed.is_some() || !complained {
                            tell(&app, "error", e);
                            complained = true;
                        }
                        ears.cancel();
                        thread::sleep(Duration::from_secs(3));
                        continue;
                    }
                }
            }
            if mic.is_none() {
                match open_mic() {
                    Ok(m) => {
                        let mut r = ears.report.lock().unwrap();
                        r.mic = m.name.clone();
                        r.problem.clear();
                        drop(r);
                        mic = Some(m)
                    }
                    Err(e) => {
                        ears.report.lock().unwrap().problem = e.clone();
                        if armed.is_some() || !complained {
                            tell(&app, "error", e);
                            complained = true;
                        }
                        ears.cancel();
                        thread::sleep(Duration::from_secs(3));
                        continue;
                    }
                }
            }
            let m = mic.as_mut().unwrap();
            match m.rx.recv_timeout(Duration::from_millis(200)) {
                Ok(chunk) => {
                    let (captured, chunk) = chunk;
                    // Audio that waited while a sentence was being understood: too old to act on.
                    if ears.is_speaking() || captured.elapsed() > Duration::from_secs(4) {
                        vad.reset();
                        continue;
                    }
                    let mut pcm = Vec::with_capacity(chunk.len() / 2);
                    m.down.process(&chunk, &mut pcm);
                    let utterances = vad.push(&pcm);
                    {
                        // "Loudest in the last half minute", for /micro.
                        let mut r = ears.report.lock().unwrap();
                        let loud = vad_peak_value(&pcm);
                        let stale = r.peak_at.map_or(true, |t| t.elapsed() > Duration::from_secs(30));
                        if stale || loud > r.peak {
                            r.peak = loud;
                            r.peak_at = Some(Instant::now());
                        }
                    }
                    for utterance in utterances {
                        handle(&app, &ears, model.as_ref().unwrap(), &utterance);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => mic = None,
            }
            if let Some(until) = ears.armed() {
                if Instant::now() > until && !vad.talking() {
                    ears.cancel();
                    tell(&app, "idle", "");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hears_its_name_even_misspelled() {
        let n = "Pyrosan";
        assert_eq!(after_wake_word("Hey Pyrosan, quel temps fait-il ?", n).as_deref(), Some("quel temps fait-il ?"));
        assert_eq!(after_wake_word("Et Pyrosan, lance LoL.", n).as_deref(), Some("lance LoL."));
        assert_eq!(after_wake_word("Yrosan, monte le son", n).as_deref(), Some("monte le son"));
        assert_eq!(after_wake_word("hé Pyro-san mets pause", n).as_deref(), Some("mets pause"));
        assert_eq!(after_wake_word("Pirosan", n).as_deref(), Some(""));
        assert_eq!(after_wake_word("et pire aux ans, quel temps fera-t-il", n), None);
        assert_eq!(after_wake_word("Je pense qu'on va faire une partie ce soir.", n), None);
        assert_eq!(after_wake_word("Paris, c'est sympa", n), None);
        assert_eq!(after_wake_word("Hey mon gars, lance League of Legends.", "mon gars").as_deref(), Some("lance League of Legends."));
        assert_eq!(after_wake_word("Mon gars ?", "mon gars").as_deref(), Some("?"));
        assert_eq!(after_any("Et Pyrosan, ça va ?", &["mon gars".into(), "Pyrosan".into()]).as_deref(), Some("ça va ?"));
        assert_eq!(after_wake_word("On y va les gars", "mon gars"), None);
    }

    #[test]
    fn invented_text_is_dropped() {
        assert_eq!(clean(" (signal)"), "");
        assert_eq!(clean("Sous-titres réalisés par la communauté d'Amara.org"), "");
        assert_eq!(clean("[Musique] Hey Pyrosan *rire* lance Discord"), "Hey Pyrosan lance Discord");
    }

    #[test]
    fn finds_speech_between_silences() {
        let mut vad = Vad::default();
        let quiet = vec![0.001f32; RATE];
        let tone: Vec<f32> = (0..RATE).map(|i| (i as f32 * 0.05).sin() * 0.2).collect();
        assert!(vad.push(&quiet).is_empty());
        assert!(vad.push(&tone).is_empty(), "still talking");
        let done = vad.push(&quiet);
        assert_eq!(done.len(), 1);
        // About the one second of voice, plus a bit before and the quiet that ended it.
        let secs = done[0].len() as f32 / RATE as f32;
        assert!((1.0..2.4).contains(&secs), "{secs}");
        // A short click is ignored.
        let click: Vec<f32> = (0..RATE / 10).map(|i| (i as f32 * 0.05).sin() * 0.2).collect();
        vad.push(&click);
        assert!(vad.push(&quiet).is_empty());
    }

    #[test]
    fn quiet_sentences_are_boosted() {
        let quiet: Vec<f32> = (0..16_000).map(|i| (i as f32 * 0.05).sin() * 0.01).collect();
        let boosted = louder(&quiet);
        let rms = (boosted.iter().map(|x| x * x).sum::<f32>() / boosted.len() as f32).sqrt();
        assert!((0.095..=0.105).contains(&rms), "brought to a normal speaking level: {rms}");
        let whisper: Vec<f32> = quiet.iter().map(|x| x * 0.1).collect();
        let rms = (louder(&whisper).iter().map(|x| x * x).sum::<f32>() / whisper.len() as f32).sqrt();
        assert!(rms < 0.03, "never more than x30: {rms}");
        let loud: Vec<f32> = quiet.iter().map(|x| x * 80.0).collect();
        assert_eq!(louder(&loud), loud, "already loud: untouched");
    }

    #[test]
    fn downsamples_to_16k() {
        let mut d = Downsampler::new(48_000);
        let mut out = Vec::new();
        d.process(&vec![0.5; 48_000], &mut out);
        assert_eq!(out.len(), 16_000);
        assert!((out[100] - 0.5).abs() < 1e-6);
    }
}

#[cfg(test)]
mod live {
    /// Real speech through the whole pipeline. Needs the model and test clips in /tmp.
    #[test]
    #[ignore]
    fn quiet_speech_is_understood() {
        let model = std::path::Path::new("resources/models/ggml-small-q5_1.bin");
        let ctx = whisper_rs::WhisperContext::new_with_params(model, whisper_rs::WhisperContextParameters::default()).unwrap();
        for clip in ["t1", "t4"] {
            let bytes = std::fs::read(format!("/tmp/voice/wav/{clip}.f32")).unwrap();
            let quiet: Vec<f32> = bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]) * 0.05).collect();
            let boosted = super::louder(&quiet);
            let raw = super::transcribe(&ctx, &boosted, "Discord, Steam, Spotify, League of Legends, Lens Studio. Hey mon gars.", false);
            println!("{clip}: {raw:?}");
            let retry = super::transcribe(&ctx, &boosted, "Hey Pyrosan.", true);
            println!("{clip} retry: {retry:?}");
        }
    }
}
