use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// User-tweakable settings, changed with `/settings <key> <value>` in the chat box.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub name: String,
    /// Model passed to `claude --model`.
    pub model: String,
    /// Multiplier on how fast stats decay (and sleep restores energy).
    pub speed: f64,
    pub on_top: bool,
    /// Max chat messages per hour.
    pub hourly_limit: usize,
    /// Sprite sheet to draw, one of `SKINS`.
    pub skin: String,
    /// Minutes between unprompted pep talks; 0 turns them off.
    pub motivate: u32,
    /// Read replies aloud (ElevenLabs when set up, otherwise the Windows voice).
    pub voice: bool,
    /// Bubble and panel colours: "dark" or "light".
    pub theme: String,
    /// Start with Windows.
    pub autostart: bool,
    /// ElevenLabs API key; stays in settings.json on this computer, never shown back.
    pub eleven_key: String,
    pub eleven_voice: String,
    /// "fast" (cheaper, quicker) or "quality".
    pub eleven_model: String,
}

/// The ElevenLabs voice picked for Pyrosan.
pub const DEFAULT_ELEVEN_VOICE: &str = "B8gJV1IhpuegLxdpXFOE";

/// Must match the sheets in public/sprites (see scripts/sprites/skins.mjs).
pub const SKINS: [&str; 6] = ["blob", "sprout", "beancat", "ember", "pyro", "spine"];

impl Default for Settings {
    fn default() -> Self {
        Self {
            name: "Mochi".into(),
            // Sonnet: the pet now searches the web and gives real answers, not just one-liners.
            model: std::env::var("CLAUDE_PET_MODEL").unwrap_or_else(|_| "sonnet".into()),
            speed: 1.0,
            on_top: true,
            hourly_limit: 30,
            skin: "blob".into(),
            motivate: 45,
            voice: true,
            theme: "dark".into(),
            autostart: true,
            eleven_key: String::new(),
            eleven_voice: DEFAULT_ELEVEN_VOICE.into(),
            eleven_model: "fast".into(),
        }
    }
}

pub const KEYS: &str = "name, skin, model, voice, theme, autostart, elevenkey, elevenvoice, elevenmodel, speed, ontop, limit, motivate";

impl Settings {
    pub fn load(dir: &Path) -> Self {
        fs::read_to_string(path(dir))
            .ok()
            .and_then(|s| serde_json::from_str::<Self>(&s).ok())
            .map(|mut s| {
                // A skin that was removed from the app falls back to the default look.
                if !SKINS.contains(&s.skin.as_str()) {
                    s.skin = Self::default().skin;
                }
                s
            })
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        fs::create_dir_all(dir)?;
        fs::write(path(dir), serde_json::to_vec_pretty(self)?)
    }

    /// What the pet is called right now: skins with their own character (Spine → Spiny)
    /// override the user's chosen name, which is kept for the other skins.
    pub fn display_name(&self) -> String {
        crate::chat::persona_name(&self.skin).map(str::to_string).unwrap_or_else(|| self.name.clone())
    }

    /// The model chat uses right now: a skin's own character may bring its own.
    pub fn effective_model(&self) -> String {
        crate::chat::persona_model(&self.skin).map(str::to_string).unwrap_or_else(|| self.model.clone())
    }

    pub fn describe(&self) -> String {
        format!(
            "name: {} · skin: {} · model: {} · voice: {}{} · theme: {} · autostart: {} · speed: {}x · ontop: {} · limit: {}/hour · motivate: {}",
            self.display_name(),
            self.skin,
            self.effective_model(),
            on_off(self.voice),
            if self.eleven_key.is_empty() { " (windows)".to_string() } else { format!(" (elevenlabs {}, {})", self.eleven_voice, self.eleven_model) },
            self.theme,
            on_off(self.autostart),
            self.speed,
            on_off(self.on_top),
            self.hourly_limit,
            if self.motivate == 0 { "off".to_string() } else { format!("every {} min", self.motivate) }
        )
    }

    /// Validates and applies one setting, returning a confirmation line.
    pub fn set(&mut self, key: &str, value: &str) -> Result<String, String> {
        let value = value.trim();
        match key {
            "name" if crate::chat::persona_name(&self.skin).is_some() => Err(format!(
                "{} doesn't do name changes. switch skin to rename your other pet",
                self.display_name().to_lowercase()
            )),
            "name" => {
                let len = value.chars().count();
                if !(1..=24).contains(&len) {
                    return Err("names are 1–24 characters".into());
                }
                self.name = value.into();
                Ok(format!("call me {value} from now on!"))
            }
            "model" => {
                let ok = ["haiku", "sonnet", "opus"].contains(&value) || value.starts_with("claude-");
                if !ok || value.contains(char::is_whitespace) {
                    return Err("model must be haiku, sonnet, opus or a full claude-… id".into());
                }
                self.model = value.into();
                Ok(format!("brain swapped to {value}"))
            }
            "speed" => {
                let speed: f64 = value.parse().map_err(|_| "speed is a number like 0.5 or 2")?;
                if !(0.0..=10.0).contains(&speed) {
                    return Err("speed must be between 0 (paused) and 10".into());
                }
                self.speed = speed;
                Ok(if speed == 0.0 { "time is frozen. stats won't change".into() } else { format!("stats now change at {speed}x speed") })
            }
            "ontop" => {
                self.on_top = parse_on_off(value).ok_or("ontop is on or off")?;
                Ok(format!("always on top: {}", on_off(self.on_top)))
            }
            "voice" => {
                self.voice = parse_on_off(value).ok_or("voice is on or off")?;
                Ok(if self.voice { "voix activée : je lis mes réponses à voix haute".into() } else { "voix coupée : je reste silencieux".into() })
            }
            "theme" => {
                self.theme = match value.to_lowercase().as_str() {
                    "dark" | "sombre" | "noir" => "dark".into(),
                    "light" | "clair" | "blanc" => "light".into(),
                    _ => return Err("theme is dark or light".into()),
                };
                Ok(format!("thème {} !", if self.theme == "dark" { "sombre" } else { "clair" }))
            }
            "autostart" => {
                self.autostart = parse_on_off(value).ok_or("autostart is on or off")?;
                Ok(if self.autostart { "je démarrerai avec Windows".into() } else { "je ne démarrerai plus avec Windows".into() })
            }
            "elevenkey" => {
                if matches!(value, "off" | "none" | "aucune") {
                    self.eleven_key.clear();
                    return Ok("clé ElevenLabs retirée : je reprends la voix Windows".into());
                }
                if !crate::tts::valid_key(value) {
                    return Err("cette clé ElevenLabs n'a pas l'air valide (copie-la en entier)".into());
                }
                self.eleven_key = value.into();
                Ok("clé ElevenLabs enregistrée sur ton PC 🔑 je parle avec ma nouvelle voix !".into())
            }
            "elevenvoice" => {
                if !crate::tts::valid_voice(value) {
                    return Err("un ID de voix ElevenLabs ressemble à B8gJV1IhpuegLxdpXFOE".into());
                }
                self.eleven_voice = value.into();
                Ok(format!("voix ElevenLabs : {value}"))
            }
            "elevenmodel" => {
                self.eleven_model = match value {
                    "fast" | "rapide" => "fast".into(),
                    "quality" | "qualite" | "qualité" => "quality".into(),
                    _ => return Err("elevenmodel is fast (cheaper) or quality".into()),
                };
                Ok(format!("modèle de voix : {}", self.eleven_model))
            }
            "limit" => {
                let limit: usize = value.parse().map_err(|_| "limit is a whole number")?;
                if !(1..=200).contains(&limit) {
                    return Err("limit must be between 1 and 200 messages per hour".into());
                }
                self.hourly_limit = limit;
                Ok(format!("up to {limit} chats per hour"))
            }
            "skin" => {
                if !SKINS.contains(&value) {
                    return Err(format!("skins: {}", SKINS.join(", ")));
                }
                let was_spine = self.skin == "spine";
                self.skin = value.into();
                Ok(match (was_spine, value == "spine") {
                    (false, true) => "ugh. fine. i'm a spine now. don't expect me to be nice to you 💀".into(),
                    (true, false) => format!("*poof* new look: {value}! (the spine says it won't miss you)"),
                    _ => format!("*poof* new look: {value}!"),
                })
            }
            "motivate" => {
                let minutes: u32 = match value {
                    "off" => 0,
                    v => v.parse().map_err(|_| "motivate is minutes between pep talks, or off")?,
                };
                if minutes > 240 {
                    return Err("motivate must be 0–240 minutes".into());
                }
                self.motivate = minutes;
                Ok(if minutes == 0 { "ok, no unprompted pep talks".into() } else { format!("i'll cheer you on every {minutes} minutes") })
            }
            _ => Err(format!("unknown setting '{key}'. try: {KEYS}")),
        }
    }
}

fn path(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

fn parse_on_off(value: &str) -> Option<bool> {
    match value.to_lowercase().as_str() {
        "on" | "true" | "yes" | "oui" => Some(true),
        "off" | "false" | "no" | "non" => Some(false),
        _ => None,
    }
}

/// Everything you can type in the chat box that starts with `/`.
#[derive(Debug, PartialEq)]
pub enum Command {
    Help,
    ShowSettings,
    Set(String, String),
    Personality { reset: bool },
    Motivate,
    Doctor,
    Stats,
    Clear,
    Reset { confirmed: bool },
    Remember(String),
    Memories,
    Forget,
    Reminders,
    Cancel(Option<usize>),
}

pub const HELP: &str = "/settings · /settings <key> <value> · /remember <fact> · /memories · /forget · /reminders · /cancel <n|all> · /motivate · /stats · /doctor · /personality [reset] · /clear · /reset · /bye · \"lance <app>\" opens an app";

pub fn parse(input: &str) -> Result<Command, String> {
    let input = input.trim().trim_start_matches('/');
    let (cmd, rest) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    let rest = rest.trim();
    match cmd.to_lowercase().as_str() {
        "help" | "?" => Ok(Command::Help),
        "settings" | "set" if rest.is_empty() => Ok(Command::ShowSettings),
        "settings" | "set" => {
            let (key, value) = rest.split_once(char::is_whitespace).ok_or("usage: /settings <key> <value>")?;
            Ok(Command::Set(key.to_lowercase(), value.trim().into()))
        }
        "personality" => Ok(Command::Personality { reset: rest == "reset" }),
        "motivate" | "pep" => Ok(Command::Motivate),
        "doctor" => Ok(Command::Doctor),
        "stats" => Ok(Command::Stats),
        "clear" => Ok(Command::Clear),
        "reset" => Ok(Command::Reset { confirmed: rest == "confirm" }),
        "remember" | "retiens" if rest.is_empty() => Err("usage: /remember <something to remember>".into()),
        "remember" | "retiens" => Ok(Command::Remember(rest.into())),
        "memories" | "memoire" | "mémoire" => Ok(Command::Memories),
        "forget" | "oublie" => Ok(Command::Forget),
        "reminders" | "rappels" | "timers" => Ok(Command::Reminders),
        "cancel" | "annule" => match rest {
            "all" | "tout" | "tous" => Ok(Command::Cancel(None)),
            n => n.parse().map(|n| Command::Cancel(Some(n))).map_err(|_| "usage: /cancel <number> or /cancel all".into()),
        },
        other => Err(format!("unknown command /{other}. try /help")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands() {
        assert_eq!(parse("/help"), Ok(Command::Help));
        assert_eq!(parse("/settings"), Ok(Command::ShowSettings));
        assert_eq!(parse("/settings  Name   Sir Blob "), Ok(Command::Set("name".into(), "Sir Blob".into())));
        assert_eq!(parse("/reset"), Ok(Command::Reset { confirmed: false }));
        assert_eq!(parse("/personality reset"), Ok(Command::Personality { reset: true }));
        assert_eq!(parse("/motivate"), Ok(Command::Motivate));
        assert_eq!(parse("/reset confirm"), Ok(Command::Reset { confirmed: true }));
        assert_eq!(parse("/remember  my cat is Tofu "), Ok(Command::Remember("my cat is Tofu".into())));
        assert_eq!(parse("/retiens j'aime LoL"), Ok(Command::Remember("j'aime LoL".into())));
        assert!(parse("/remember").is_err());
        assert_eq!(parse("/memories"), Ok(Command::Memories));
        assert_eq!(parse("/forget"), Ok(Command::Forget));
        assert_eq!(parse("/rappels"), Ok(Command::Reminders));
        assert_eq!(parse("/cancel 2"), Ok(Command::Cancel(Some(2))));
        assert_eq!(parse("/cancel all"), Ok(Command::Cancel(None)));
        assert!(parse("/cancel").is_err());
        assert!(parse("/settings name").is_err());
        assert!(parse("/dance").is_err());
    }

    #[test]
    fn validates_settings() {
        let mut s = Settings::default();
        assert!(s.set("name", "Pip").is_ok());
        assert_eq!(s.name, "Pip");
        assert!(s.set("name", &"x".repeat(30)).is_err());
        assert!(s.set("model", "sonnet").is_ok());
        assert!(s.set("model", "gpt-4").is_err());
        assert!(s.set("speed", "2.5").is_ok());
        assert_eq!(s.speed, 2.5);
        assert!(s.set("speed", "-1").is_err());
        assert!(s.set("speed", "fast").is_err());
        assert!(s.set("ontop", "off").is_ok());
        assert!(!s.on_top);
        assert!(s.voice, "voice is on by default");
        assert!(s.set("voice", "off").is_ok());
        assert!(!s.voice);
        assert!(s.set("voice", "Oui").is_ok());
        assert!(s.voice);
        assert!(s.set("voice", "loud").is_err());
        assert_eq!(s.theme, "dark");
        assert!(s.set("theme", "clair").is_ok());
        assert_eq!(s.theme, "light");
        assert!(s.set("theme", "pink").is_err());
        assert!(s.set("elevenkey", "sk_abcdef0123456789").is_ok());
        assert!(!s.describe().contains("sk_abcdef"), "the key is never shown");
        assert!(s.set("elevenkey", "not a key").is_err());
        assert!(s.set("elevenvoice", "../x").is_err());
        assert!(s.set("elevenmodel", "quality").is_ok());
        assert!(s.set("elevenkey", "off").is_ok());
        assert!(s.eleven_key.is_empty());
        assert!(s.set("skin", "pyro").is_ok());
        assert_eq!(s.display_name(), "Pip", "pyro uses the usual name and personality");
        assert!(s.set("limit", "0").is_err());
        assert!(s.set("skin", "ember").is_ok());
        assert!(s.set("skin", "spine").is_ok());
        assert_eq!(s.display_name(), "Spiny");
        assert_eq!(s.effective_model(), "sonnet");
        assert!(s.set("name", "Bob").is_err(), "spiny can't be renamed");
        assert!(s.set("skin", "blob").is_ok());
        assert_eq!(s.display_name(), "Pip", "the usual name comes back");
        assert!(s.set("skin", "dragon").is_err());
        assert!(s.set("motivate", "off").is_ok());
        assert_eq!(s.motivate, 0);
        assert!(s.set("motivate", "999").is_err());
        assert!(s.set("colour", "red").is_err());
    }

    #[test]
    fn round_trips_and_fills_missing_fields() {
        let dir = std::env::temp_dir().join(format!("claude-pet-settings-{}", crate::stats::now_ms()));
        let mut s = Settings::default();
        s.set("name", "Pip").unwrap();
        s.save(&dir).unwrap();
        assert_eq!(Settings::load(&dir), s);
        fs::write(dir.join("settings.json"), r#"{"name":"Old"}"#).unwrap();
        let old = Settings::load(&dir);
        assert_eq!((old.name.as_str(), old.speed), ("Old", 1.0));
        assert!(old.voice, "settings from before voice existed get it switched on");
        fs::write(dir.join("settings.json"), r#"{"name":"Old","skin":"spark"}"#).unwrap();
        assert_eq!(Settings::load(&dir).skin, "blob", "a removed skin falls back to the default");
        fs::remove_dir_all(dir).unwrap();
    }
}
