use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

/// Seeded into the brain folder on first run; the user's edited copy wins after that.
const DEFAULT_PERSONALITY: &str = include_str!("../personality/CLAUDE.md");
const MAX_MESSAGE_CHARS: usize = 2000;
const HISTORY_TURNS: usize = 12;
/// Older turns are kept shorter so the prompt (passed on the command line, which Windows
/// caps at 32k characters) stays small however long the conversation gets.
const HISTORY_HUMAN_CHARS: usize = 400;
const HISTORY_PET_CHARS: usize = 900;
/// Web searches take a while, so give the brain time to look things up.
const CLI_TIMEOUT: Duration = Duration::from_secs(180);
/// Long-term notes the human asked the pet to remember (`/remember …`).
pub const MAX_NOTES: usize = 20;
const NOTE_CHARS: usize = 200;
/// The only tools the pet gets: it can look things up, never touch the computer.
const TOOLS: &str = "WebSearch,WebFetch";
/// Stops a runaway search loop; plenty for a few searches and page reads.
const MAX_TURNS: &str = "12";

/// Allows at most `max_calls` per sliding `window`, with at least `min_gap` between calls.
pub struct RateLimiter {
    min_gap: Duration,
    window: Duration,
    max_calls: usize,
    calls: VecDeque<Instant>,
}

impl RateLimiter {
    pub fn new(min_gap: Duration, max_calls: usize, window: Duration) -> Self {
        Self { min_gap, window, max_calls, calls: VecDeque::new() }
    }

    pub fn set_max_calls(&mut self, max_calls: usize) {
        self.max_calls = max_calls;
    }

    /// Records a call and returns Ok, or returns how long until the next call is allowed.
    pub fn try_acquire(&mut self, now: Instant) -> Result<(), Duration> {
        while self.calls.front().is_some_and(|t| now.duration_since(*t) >= self.window) {
            self.calls.pop_front();
        }
        let gap_wait = self
            .calls
            .back()
            .map(|last| self.min_gap.saturating_sub(now.duration_since(*last)))
            .unwrap_or_default();
        let window_wait = if self.calls.len() >= self.max_calls {
            self.window - now.duration_since(self.calls[0])
        } else {
            Duration::ZERO
        };
        let wait = gap_wait.max(window_wait);
        if !wait.is_zero() {
            return Err(wait);
        }
        self.calls.push_back(now);
        Ok(())
    }
}

fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

/// Reads JSON from `path`, or the default when it's missing or unreadable.
pub(crate) fn load_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Writes to a temp file first so a crash mid-write can't corrupt the file.
pub(crate) fn save_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(tmp, path)
}

/// Rolling memory of the last exchanges so the pet can follow a conversation.
/// Saved next to the pet, so it survives restarts until `/clear`.
#[derive(Default, Serialize, Deserialize)]
pub struct History {
    turns: VecDeque<(String, String)>,
}

impl History {
    pub fn push(&mut self, human: String, pet: String) {
        self.turns.push_back((shorten(&human, HISTORY_HUMAN_CHARS), shorten(&pet, HISTORY_PET_CHARS)));
        while self.turns.len() > HISTORY_TURNS {
            self.turns.pop_front();
        }
    }

    pub fn clear(&mut self) {
        self.turns.clear();
    }

    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("history.json")
    }

    pub fn load(data_dir: &Path) -> Self {
        let mut history: Self = load_json(&Self::path(data_dir));
        while history.turns.len() > HISTORY_TURNS {
            history.turns.pop_front();
        }
        history
    }

    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        save_json(&Self::path(data_dir), self)
    }
}

/// Things the human asked the pet to remember (`items`) and lasting facts the pet noticed
/// on its own (`auto`), sent with every message.
#[derive(Default, Serialize, Deserialize)]
pub struct Notes {
    items: Vec<String>,
    #[serde(default)]
    auto: Vec<String>,
}

/// How many facts the pet keeps on its own; the oldest go first.
pub const MAX_AUTO_NOTES: usize = 30;

impl Notes {
    fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("notes.json")
    }

    pub fn load(data_dir: &Path) -> Self {
        load_json(&Self::path(data_dir))
    }

    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        save_json(&Self::path(data_dir), self)
    }

    pub fn add(&mut self, note: &str) -> Result<(), String> {
        let note = note.trim();
        if note.is_empty() {
            return Err("usage: /remember <something to remember>".into());
        }
        if self.items.len() >= MAX_NOTES {
            return Err(format!("my memory is full ({MAX_NOTES} notes). /forget to start over"));
        }
        self.items.push(shorten(note, NOTE_CHARS));
        Ok(())
    }

    /// A fact the pet decided to remember. Returns false if it already knew it.
    pub fn add_auto(&mut self, fact: &str) -> bool {
        let fact = shorten(fact.trim(), NOTE_CHARS);
        let key = crate::launcher::normalize(&fact);
        if key.is_empty() || self.items.iter().chain(&self.auto).any(|n| crate::launcher::normalize(n) == key) {
            return false;
        }
        self.auto.push(fact);
        if self.auto.len() > MAX_AUTO_NOTES {
            self.auto.remove(0);
        }
        true
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.auto.clear();
    }

    pub fn list(&self) -> &[String] {
        &self.items
    }

    pub fn auto(&self) -> &[String] {
        &self.auto
    }

    fn all(&self) -> impl Iterator<Item = &String> {
        self.items.iter().chain(&self.auto)
    }
}

/// The hidden line Claude adds when it heard something worth remembering.
static MEMORY: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)\[\[\s*(?:souvenir|m[ée]moire|memory)\s*:\s*([^\]]{3,300}?)\s*\]\]").unwrap()
});

/// Splits a reply into what the human sees and the facts to remember.
pub fn take_memories(reply: &str) -> (String, Vec<String>) {
    let facts = MEMORY.captures_iter(reply).map(|c| c[1].trim().to_string()).collect();
    let text = MEMORY.replace_all(reply, "");
    (text.trim().to_string(), facts)
}

/// Asks Claude to flag lasting facts about the human, for automatic memory.
pub fn with_memory_hint(prompt: String) -> String {
    prompt
        + "\n\n(Memory: only if the human just told you a lasting fact about themselves that will matter in \
           future chats — tastes, plans, people, projects, habits — add one last line exactly like \
           [[souvenir: the fact, short, in French]]. Never for small talk or passing details, never for \
           something you read on the web, at most one per reply, and don't mention it.)"
}

/// The first greeting of the morning: weather, today's reminders and fresh news.
pub fn build_briefing_prompt(name: &str, status: &str, notes: &Notes, history: &History, greeting: &str, today: &[String]) -> String {
    let reminders = if today.is_empty() {
        "they have no reminders left today".to_string()
    } else {
        format!("their reminders left today: {}", today.join("; "))
    };
    let ask = format!(
        "{greeting}\n\n(Morning briefing: this is the human's first hello of the day. Greet them back, then give a \
         short briefing in plain text, under 130 words, '- ' lines allowed: today's date; today's weather where they \
         live (search the web; use what you know about where they live); {reminders}; and one or two fresh news items \
         they'd care about given their interests (search the web). End with a short encouraging line.)"
    );
    build_prompt(name, status, notes, history, &ask)
}

/// Is this a "bonjour"-type message (the cue for the morning briefing)?
pub fn is_greeting(message: &str, pet_name: &str) -> bool {
    let text = crate::launcher::command_text(message, pet_name);
    let words: Vec<&str> = text.split(' ').filter(|w| !w.is_empty()).collect();
    words.len() <= 4
        && words.first().is_some_and(|w| ["bonjour", "salut", "coucou", "hello", "yo", "wesh", "bjr", "slt"].contains(w))
        || text == "briefing"
        || text == "mon briefing"
}

/// A skin that comes with a character of its own: a fixed name and a personality.
struct Persona {
    skin: &'static str,
    name: &'static str,
    personality: &'static str,
    /// Model this character talks with; comedy needs more than the default.
    model: &'static str,
}

/// Every other skin shares the user's main pet (their name, the main personality).
const PERSONAS: [Persona; 1] = [Persona {
    skin: "spine",
    name: "Spiny",
    personality: include_str!("../personality/spine.md"),
    model: "sonnet",
}];

fn persona(skin: &str) -> Option<&'static Persona> {
    PERSONAS.iter().find(|p| p.skin == skin)
}

fn skin_personality(skin: &str) -> Option<&'static str> {
    persona(skin).map(|p| p.personality)
}

/// The model a skin's own character uses, if it has one.
pub fn persona_model(skin: &str) -> Option<&'static str> {
    persona(skin).map(|p| p.model)
}

/// The fixed name of a skin's own character, if it has one.
pub fn persona_name(skin: &str) -> Option<&'static str> {
    persona(skin).map(|p| p.name)
}

/// Whether picking this skin swaps in its own personality.
pub fn has_own_personality(skin: &str) -> bool {
    skin_personality(skin).is_some()
}

/// The built-in personality for a skin.
pub fn default_personality(skin: &str) -> &'static str {
    skin_personality(skin).unwrap_or(DEFAULT_PERSONALITY)
}

/// The folder `claude` runs in for this skin. Claude Code picks up its CLAUDE.md as the
/// pet's personality. Skins with their own personality get their own folder, so switching
/// skins never overwrites the user's edits to another one.
pub fn brain_dir(data_dir: &Path, skin: &str) -> std::io::Result<PathBuf> {
    let dir = if has_own_personality(skin) { data_dir.join(format!("brain-{skin}")) } else { data_dir.join("brain") };
    fs::create_dir_all(&dir)?;
    let file = dir.join("CLAUDE.md");
    // A copy of what we wrote, so a personality nobody edited can follow app updates.
    let seeded = dir.join(".seeded.md");
    let default = default_personality(skin);
    let untouched = match (fs::read_to_string(&file), fs::read_to_string(&seeded)) {
        (Err(_), _) => true,
        (Ok(current), Ok(original)) => current == original,
        (Ok(_), Err(_)) => false, // from before we kept a copy: assume it was edited
    };
    if untouched && fs::read_to_string(&file).ok().as_deref() != Some(default) {
        fs::write(&file, default)?;
        fs::write(&seeded, default)?;
    }
    Ok(dir)
}

/// Overwrites the personality with the skin's built-in default (which then follows updates).
pub fn reset_personality(brain: &Path, skin: &str) -> std::io::Result<()> {
    fs::write(brain.join("CLAUDE.md"), default_personality(skin))?;
    fs::write(brain.join(".seeded.md"), default_personality(skin))
}

/// Prompt for an unprompted pep talk.
pub fn build_nudge_prompt(name: &str, status: &str, notes: &Notes, history: &History) -> String {
    build_prompt(
        name,
        status,
        notes,
        history,
        "(nobody said anything: the human is busy working at their computer. Give them a short, \
         specific nudge of motivation to keep going, or if it's been a while, a gentle reminder to \
         stretch or drink water. Don't start with a greeting.)",
    )
}

/// Used when Claude is unavailable or rate-limited, so pep talks never go silent.
pub fn fallback_pep(seed: u64) -> &'static str {
    const LINES: [&str; 8] = [
        "you've got this! one small step at a time",
        "tiny progress is still progress. keep going!",
        "deep breath, sip of water, back at it. i believe in you",
        "focus mode: on. i'll guard the desktop *boing*",
        "stretch break? then let's crush the next task",
        "proud of you for showing up today",
        "pick the smallest next step and do just that one",
        "future you is gonna be so happy you did this",
    ];
    LINES[(seed % LINES.len() as u64) as usize]
}

pub fn clean_message(raw: &str) -> Result<String, String> {
    let msg = raw.trim();
    if msg.is_empty() {
        return Err("say something first!".into());
    }
    Ok(msg.chars().take(MAX_MESSAGE_CHARS).collect())
}

pub fn build_prompt(name: &str, status: &str, notes: &Notes, history: &History, message: &str) -> String {
    let now = crate::reminders::local_now_text(chrono::Local::now());
    let mut prompt = format!("[your name] {name}\n[local time] {now}\n[your current state] {status}\n");
    if notes.all().next().is_some() {
        prompt.push_str("[things the human asked you to remember]\n");
        for note in notes.all() {
            prompt.push_str(&format!("- {note}\n"));
        }
    }
    if !history.turns.is_empty() {
        prompt.push_str("[recent conversation]\n");
        for (human, pet) in &history.turns {
            prompt.push_str(&format!("human: {human}\nyou: {pet}\n"));
        }
    }
    prompt.push_str(&format!("[human says]\n{message}\n\nReply in character as the pet."));
    prompt
}

/// `claude.exe` from PATH, or the native installer's location when the app was
/// launched without the user's shell PATH. CLAUDE_PET_CLI overrides both.
fn cli_path() -> PathBuf {
    if let Some(p) = std::env::var_os("CLAUDE_PET_CLI") {
        return p.into();
    }
    let local = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(|home| PathBuf::from(home).join(".local").join("bin").join(if cfg!(windows) { "claude.exe" } else { "claude" }));
    match local {
        Some(p) if p.exists() => p,
        _ => "claude".into(),
    }
}

pub const INSTALL_HELP: &str = "i need Claude Code to think! install it from claude.com/claude-code, \
     run `claude` once in a terminal to log in, then type /doctor";
const LOGIN_HELP: &str = "my brain isn't logged in. open a terminal, run `claude` and log in, then try again";

/// A `claude` process that never flashes a console window.
fn cli_command() -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(cli_path());
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Checks the CLI is installed, for `/doctor` and the first-run check.
pub async fn check_cli() -> Result<String, String> {
    let mut cmd = cli_command();
    cmd.arg("--version");
    match tokio::time::timeout(Duration::from_secs(20), cmd.output()).await {
        Ok(Ok(out)) if out.status.success() => {
            let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
            Ok(format!("brain found: claude {version}. if chat fails, run `claude` in a terminal to log in"))
        }
        _ => Err(INSTALL_HELP.into()),
    }
}

/// What the brain may use on one call.
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// Web search and reading web pages.
    Web,
    /// Only reading the screenshot in `screens/` (no web, so the screen can't be sent anywhere).
    Screen,
}

/// Claude's reply, and whether it used any tool to get there.
#[derive(Debug)]
pub struct Answer {
    pub text: String,
    pub used_tools: bool,
}

/// Reads `claude --output-format json`; falls back to plain text if it isn't JSON.
fn parse_answer(stdout: &str) -> Result<Answer, String> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(stdout) else {
        return Ok(Answer { text: stdout.to_string(), used_tools: true });
    };
    let text = json["result"].as_str().unwrap_or_default().trim().to_string();
    if json["is_error"].as_bool() == Some(true) || text.is_empty() {
        let why = if text.is_empty() { json["subtype"].as_str().unwrap_or("no answer").to_string() } else { text };
        return Err(format!("brain fog… ({why})"));
    }
    Ok(Answer { text, used_tools: json["num_turns"].as_u64().unwrap_or(2) > 1 })
}

/// Runs `claude -p <prompt>` in the brain folder. In `Web` mode the only tools are web search
/// and reading web pages; in `Screen` mode only reading the screenshot. No shell, no other
/// files, no MCP servers, and anything else is refused without asking (`dontAsk`), so the pet
/// can look things up but never act on the computer.
pub async fn ask_claude(brain: &Path, model: &str, prompt: &str, mode: Mode) -> Result<Answer, String> {
    let (tools, allowed) = match mode {
        Mode::Web => (TOOLS, TOOLS),
        Mode::Screen => ("Read", "Read(./screens/**)"),
    };
    let mut cmd = cli_command();
    cmd.arg("-p")
        .arg(prompt)
        .args(["--model", model])
        .args(["--output-format", "json"])
        .args(["--tools", tools])
        .args(["--allowedTools", allowed])
        .args(["--permission-mode", "dontAsk"])
        .args(["--max-turns", MAX_TURNS])
        .arg("--strict-mcp-config")
        .arg("--no-session-persistence")
        // Skip the user's global settings (hooks, plugins) so they can't leak into replies.
        .args(["--setting-sources", "project,local"])
        .current_dir(brain);

    let output = tokio::time::timeout(CLI_TIMEOUT, cmd.output())
        .await
        .map_err(|_| "i zoned out… (claude took too long)".to_string())?
        .map_err(|_| INSTALL_HELP.to_string())?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() && stdout.starts_with('{') {
        return parse_answer(&stdout);
    }
    if !output.status.success() || stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.lines().chain(stdout.lines()).find(|l| !l.trim().is_empty()).unwrap_or("no output");
        let lower = detail.to_lowercase();
        if ["login", "log in", "api key", "auth"].iter().any(|k| lower.contains(k)) {
            return Err(LOGIN_HELP.into());
        }
        return Err(format!("brain fog… ({detail})"));
    }
    parse_answer(&stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter() -> RateLimiter {
        RateLimiter::new(Duration::from_secs(5), 3, Duration::from_secs(60))
    }

    #[test]
    fn enforces_minimum_gap() {
        let mut rl = limiter();
        let t0 = Instant::now();
        assert!(rl.try_acquire(t0).is_ok());
        assert_eq!(rl.try_acquire(t0 + Duration::from_secs(2)), Err(Duration::from_secs(3)));
        assert!(rl.try_acquire(t0 + Duration::from_secs(5)).is_ok());
    }

    #[test]
    fn enforces_window_quota() {
        let mut rl = limiter();
        let t0 = Instant::now();
        for i in 0..3 {
            assert!(rl.try_acquire(t0 + Duration::from_secs(10 * i)).is_ok());
        }
        assert_eq!(rl.try_acquire(t0 + Duration::from_secs(30)), Err(Duration::from_secs(30)));
        assert!(rl.try_acquire(t0 + Duration::from_secs(60)).is_ok());
    }

    #[test]
    fn rejected_calls_do_not_count() {
        let mut rl = limiter();
        let t0 = Instant::now();
        rl.try_acquire(t0).unwrap();
        let _ = rl.try_acquire(t0 + Duration::from_secs(1));
        assert!(rl.try_acquire(t0 + Duration::from_secs(5)).is_ok());
    }

    #[test]
    fn history_keeps_only_recent_turns() {
        let mut h = History::default();
        for i in 0..20 {
            h.push(format!("q{i}"), format!("a{i}"));
        }
        assert_eq!(h.turns.len(), HISTORY_TURNS);
        assert_eq!(h.turns[0].0, format!("q{}", 20 - HISTORY_TURNS));
    }

    #[test]
    fn long_turns_are_shortened_in_history() {
        let mut h = History::default();
        h.push("é".repeat(5000), "x".repeat(5000));
        assert_eq!(h.turns[0].0.chars().count(), HISTORY_HUMAN_CHARS + 1);
        assert_eq!(h.turns[0].1.chars().count(), HISTORY_PET_CHARS + 1);
        assert!(h.turns[0].1.ends_with('…'));
    }

    #[test]
    fn history_and_notes_survive_restarts() {
        let dir = std::env::temp_dir().join(format!("claude-pet-history-{}", crate::stats::now_ms()));
        let mut h = History::default();
        h.push("salut".into(), "coucou !".into());
        h.save(&dir).unwrap();
        assert_eq!(History::load(&dir).turns, h.turns);

        let mut n = Notes::default();
        assert!(n.add("  ").is_err());
        n.add("j'adore League of Legends").unwrap();
        n.save(&dir).unwrap();
        assert_eq!(Notes::load(&dir).list(), ["j'adore League of Legends"]);
        for i in 1..MAX_NOTES {
            n.add(&format!("note {i}")).unwrap();
        }
        assert!(n.add("one too many").is_err());

        fs::write(dir.join("history.json"), "not json").unwrap();
        assert!(History::load(&dir).turns.is_empty(), "a corrupt file starts fresh");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prompt_includes_state_history_and_message() {
        let mut h = History::default();
        h.push("hi".into(), "hello!".into());
        let p = build_prompt("Pip", "food 50/100", &Notes::default(), &h, "how are you?");
        assert!(p.starts_with("[your name] Pip\n[local time] "));
        assert!(p.contains("\n[your current state] food 50/100"));
        assert!(!p.contains("[things the human asked you to remember]"));
        assert!(p.contains("human: hi\nyou: hello!"));
        assert!(p.contains("[human says]\nhow are you?"));

        let mut n = Notes::default();
        n.add("my cat is called Tofu").unwrap();
        let p = build_prompt("Pip", "food 50/100", &n, &h, "what's my cat's name?");
        assert!(p.contains("[things the human asked you to remember]\n- my cat is called Tofu\n"));
    }

    #[test]
    fn memories_are_taken_out_of_replies() {
        let (text, facts) = take_memories("Trop bien, Ekko mid c'est solide !\n[[souvenir: joue Ekko mid sur LoL]]");
        assert_eq!(text, "Trop bien, Ekko mid c'est solide !");
        assert_eq!(facts, ["joue Ekko mid sur LoL"]);
        assert_eq!(take_memories("rien à retenir").1.len(), 0);

        let mut n = Notes::default();
        assert!(n.add_auto("joue Ekko mid sur LoL"));
        assert!(!n.add_auto("Joue Ekko mid sur LoL !"), "already known");
        for i in 0..MAX_AUTO_NOTES {
            n.add_auto(&format!("fait numéro {i}"));
        }
        assert_eq!(n.auto().len(), MAX_AUTO_NOTES);
        assert_eq!(n.auto()[0], "fait numéro 0", "the oldest was dropped");
        let p = build_prompt("Pip", "ok", &n, &History::default(), "salut");
        assert!(p.contains("- fait numéro 5\n"));
    }

    #[test]
    fn reads_claude_json_answers() {
        let a = parse_answer(r#"{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"Salut !"}"#).unwrap();
        assert_eq!((a.text.as_str(), a.used_tools), ("Salut !", false));
        let a = parse_answer(r#"{"is_error":false,"num_turns":3,"result":"Il fera beau."}"#).unwrap();
        assert!(a.used_tools);
        assert!(parse_answer(r#"{"is_error":true,"subtype":"error_max_turns","result":""}"#).unwrap_err().contains("error_max_turns"));
        assert_eq!(parse_answer("plain text").unwrap().text, "plain text");
    }

    #[test]
    fn spots_greetings() {
        assert!(is_greeting("Bonjour Pyrosan !", "Pyrosan"));
        assert!(is_greeting("salut", "Pyrosan"));
        assert!(is_greeting("briefing", "Pyrosan"));
        assert!(!is_greeting("salut, tu peux me dire quel temps il fera demain à Montpellier ?", "Pyrosan"));
        assert!(!is_greeting("quel temps fait-il ?", "Pyrosan"));
    }

    #[test]
    fn messages_are_trimmed_and_capped() {
        assert!(clean_message("   ").is_err());
        assert_eq!(clean_message("  hey ").unwrap(), "hey");
        assert_eq!(clean_message(&"x".repeat(5000)).unwrap().len(), MAX_MESSAGE_CHARS);
    }

    #[test]
    fn brain_is_seeded_once_and_user_edits_survive() {
        let dir = std::env::temp_dir().join(format!("claude-pet-brain-{}", crate::stats::now_ms()));
        let brain = brain_dir(&dir, "blob").unwrap();
        let file = brain.join("CLAUDE.md");
        assert_eq!(fs::read_to_string(&file).unwrap(), DEFAULT_PERSONALITY);
        fs::write(&file, "custom").unwrap();
        brain_dir(&dir, "blob").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "custom");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn untouched_personalities_follow_updates_but_edits_are_kept() {
        let dir = std::env::temp_dir().join(format!("claude-pet-upgrade-{}", crate::stats::now_ms()));
        let brain = brain_dir(&dir, "blob").unwrap();
        let file = brain.join("CLAUDE.md");
        // Simulate an older app version having seeded different text.
        fs::write(&file, "old default").unwrap();
        fs::write(brain.join(".seeded.md"), "old default").unwrap();
        brain_dir(&dir, "blob").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), DEFAULT_PERSONALITY, "unedited → upgraded");

        fs::write(&file, "my own words").unwrap();
        brain_dir(&dir, "blob").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "my own words", "edited → kept");

        // Files from before the copy existed are treated as edited.
        fs::remove_file(brain.join(".seeded.md")).unwrap();
        fs::write(&file, "legacy").unwrap();
        brain_dir(&dir, "blob").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "legacy");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn spine_has_its_own_personality_and_others_share_one() {
        let dir = std::env::temp_dir().join(format!("claude-pet-skins-{}", crate::stats::now_ms()));
        let main = brain_dir(&dir, "blob").unwrap();
        assert_eq!(brain_dir(&dir, "sprout").unwrap(), main, "ordinary skins share a brain");
        let spine = brain_dir(&dir, "spine").unwrap();
        assert_ne!(spine, main);
        let text = fs::read_to_string(spine.join("CLAUDE.md")).unwrap();
        assert!(text.contains("You are Spiny"));

        // Editing one never touches the other, and reset restores the right default.
        fs::write(main.join("CLAUDE.md"), "my edits").unwrap();
        reset_personality(&spine, "spine").unwrap();
        assert_eq!(fs::read_to_string(main.join("CLAUDE.md")).unwrap(), "my edits");
        assert_eq!(fs::read_to_string(spine.join("CLAUDE.md")).unwrap(), default_personality("spine"));
        fs::remove_dir_all(dir).unwrap();
    }

    /// Talks to the real CLI. Run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn real_claude_replies_in_character() {
        let dir = std::env::temp_dir().join(format!("claude-pet-live-{}", crate::stats::now_ms()));
        let brain = brain_dir(&dir, "blob").unwrap();
        let prompt = build_prompt(
            "Mochi",
            "food 20/100 (low = starving), mood 60/100, energy 70/100, awake",
            &Notes::default(),
            &History::default(),
            "hi! what's your name?",
        );
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let reply = rt.block_on(ask_claude(&brain, "haiku", &prompt, Mode::Web)).unwrap().text;
        println!("mochi: {reply}");
        assert!(reply.to_lowercase().contains("mochi"));
        fs::remove_dir_all(dir).unwrap();
    }
}
