//! "lance League of Legends": opens an app from the human's own Start menu and desktop
//! shortcuts. Only the message the human typed (or dictated) can trigger it, never
//! Claude's reply, so nothing the pet reads on the web can make it launch anything.

use std::{
    fs,
    path::{Path, PathBuf},
};

/// Words that ask to open something.
/// ("joue" is left out on purpose: "joue avec moi" is for the pet, not an app.)
const VERBS: &[&str] = &[
    "lance", "lances", "lancer", "ouvre", "ouvres", "ouvrir", "demarre", "demarres", "demarrer", "launch",
    "open", "start", "run",
];

/// Polite or addressing words allowed before the verb: "hey pyro, tu peux lancer…".
pub(crate) const PREAMBLE: &[&str] = &[
    "hey", "he", "et", "ok", "okay", "eh", "dis", "pyro", "pyrosan", "jarvis", "tu", "peux", "pourrais",
    "est", "ce", "que", "stp", "svp", "please", "can", "could", "you", "vas", "y",
];

/// Politeness that can trail a command: "monte le son stp".
const TRAILING: &[&str] = &["stp", "svp", "please", "merci", "maintenant", "now"];

/// Little words around the app's name: "lance-moi le jeu League of Legends stp".
const FILLERS: &[&str] = &[
    "moi", "nous", "le", "la", "les", "l", "un", "une", "mon", "ma", "mes", "a", "au", "jeu", "appli",
    "application", "logiciel", "programme", "the", "my", "me", "app", "game", "stp", "svp", "please",
];

/// More words than this after the verb is a sentence, not an app name: leave it to Claude.
const MAX_NAME_WORDS: usize = 5;

/// Shortcuts whose name contains one of these are never what someone wants to launch.
const SKIP: &[&str] = &[
    "uninstall", "desinstall", "readme", "lisez", "manual", "manuel", "website", "site web", "support",
];

/// Lowercase, no accents, letters and digits only, single spaces.
pub(crate) fn normalize(text: &str) -> String {
    let mapped: String = text
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' | 'á' | 'ã' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' | 'í' => 'i',
            'ô' | 'ö' | 'ó' | 'õ' => 'o',
            'ù' | 'û' | 'ü' | 'ú' => 'u',
            'ç' => 'c',
            'ÿ' => 'y',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The words of a command once "hey pyrosan, tu peux…" in front and "stp" behind are dropped,
/// normalized: "Pyrosan, monte le son stp !" → "monte le son".
pub(crate) fn command_text(message: &str, pet_name: &str) -> String {
    let text = normalize(message);
    let pet = normalize(pet_name);
    let mut words: Vec<&str> = text.split(' ').filter(|w| !w.is_empty()).collect();
    let skip = words.iter().take_while(|w| PREAMBLE.contains(w) || **w == pet).count();
    words.drain(..skip);
    while words.last().is_some_and(|w| TRAILING.contains(w) || *w == pet) {
        words.pop();
    }
    words.join(" ")
}

/// The app name in "lance league of legends", or None if the message isn't a launch request.
/// `pet_name` is allowed before the verb too ("Pyro, ouvre Discord").
pub fn requested_app(message: &str, pet_name: &str) -> Option<String> {
    let text = normalize(message);
    let words: Vec<&str> = text.split(' ').filter(|w| !w.is_empty()).collect();
    let pet = normalize(pet_name);
    let verb = words.iter().position(|w| VERBS.contains(w))?;
    if !words[..verb].iter().all(|w| PREAMBLE.contains(w) || *w == pet) {
        return None;
    }
    let mut rest = &words[verb + 1..];
    while let Some((first, tail)) = rest.split_first() {
        if !FILLERS.contains(first) && !VERBS.contains(first) {
            break;
        }
        rest = tail;
    }
    while let Some((last, head)) = rest.split_last() {
        if !FILLERS.contains(last) && *last != pet && !TRAILING.contains(last) {
            break;
        }
        rest = head;
    }
    if rest.is_empty() || rest.len() > MAX_NAME_WORDS {
        return None;
    }
    Some(rest.join(" "))
}

/// Where Windows keeps the human's own shortcuts: the Start menu (theirs and everyone's)
/// and the desktop (including a OneDrive one).
pub fn shortcut_dirs() -> Vec<PathBuf> {
    let env = |key: &str| std::env::var_os(key).map(PathBuf::from);
    let start_menu = ["Microsoft", "Windows", "Start Menu", "Programs"];
    let mut dirs = Vec::new();
    for base in [env("APPDATA"), env("ProgramData")].into_iter().flatten() {
        dirs.push(start_menu.iter().fold(base, |p, part| p.join(part)));
    }
    for (key, sub) in [("USERPROFILE", "Desktop"), ("OneDrive", "Desktop"), ("OneDrive", "Bureau"), ("PUBLIC", "Desktop")] {
        if let Some(base) = env(key) {
            dirs.push(base.join(sub));
        }
    }
    dirs.retain(|d| d.is_dir());
    dirs
}

/// Every .lnk / .url shortcut under `dir`, as (name, path).
fn collect(dir: &Path, depth: usize, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                collect(&path, depth - 1, out);
            }
            continue;
        }
        let is_shortcut = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("lnk") || e.eq_ignore_ascii_case("url"));
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let lower = normalize(name);
        if is_shortcut && !SKIP.iter().any(|s| lower.contains(s)) {
            out.push((name.to_string(), path));
        }
    }
}

/// How well a name (shortcut, window, program) matches what was asked for; 0 means not at all.
pub(crate) fn score(name: &str, wanted: &str) -> u32 {
    let n = normalize(name);
    let q = normalize(wanted);
    if q.is_empty() {
        return 0;
    }
    let compact = |s: &str| s.replace(' ', "");
    let initials: String = n.split(' ').filter_map(|w| w.chars().next()).collect();
    if n == q || compact(&n) == compact(&q) {
        100
    } else if q.len() >= 2 && initials == compact(&q) {
        90 // "lol" → League of Legends
    } else if n.starts_with(&q) {
        80
    } else if q.split(' ').all(|w| n.split(' ').any(|nw| nw.starts_with(w))) {
        70
    } else if q.len() >= 3 && compact(&n).contains(&compact(&q)) {
        60
    } else {
        0
    }
}

/// The best-matching shortcut in `dirs`; a shorter name wins a tie
/// ("League of Legends" over "League of Legends PBE").
pub fn find(wanted: &str, dirs: &[PathBuf]) -> Option<(String, PathBuf)> {
    let mut shortcuts = Vec::new();
    for dir in dirs {
        collect(dir, 4, &mut shortcuts);
    }
    shortcuts
        .into_iter()
        .map(|(name, path)| (score(&name, wanted), name, path))
        .filter(|(s, _, _)| *s > 0)
        .max_by(|a, b| a.0.cmp(&b.0).then(b.1.len().cmp(&a.1.len())))
        .map(|(_, name, path)| (name, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_launch_requests() {
        let ask = |m: &str| requested_app(m, "Pyro");
        assert_eq!(ask("lance League of Legends").as_deref(), Some("league of legends"));
        assert_eq!(ask("Lance-moi LoL.").as_deref(), Some("lol"));
        assert_eq!(ask("Pyro, ouvre Discord stp").as_deref(), Some("discord"));
        assert_eq!(ask("hey pyro tu peux lancer le jeu Hunt Showdown ?").as_deref(), Some("hunt showdown"));
        assert_eq!(ask("est-ce que tu peux ouvrir l'appli Spotify merci").as_deref(), Some("spotify"));
        assert_eq!(ask("démarre Steam").as_deref(), Some("steam"));
        assert_eq!(ask("open spotify please").as_deref(), Some("spotify"));
    }

    #[test]
    fn leaves_normal_chat_to_claude() {
        let ask = |m: &str| requested_app(m, "Pyro");
        assert_eq!(ask("quel temps demain à Sète ?"), None);
        assert_eq!(ask("j'ai lancé une partie hier, on a gagné"), None);
        assert_eq!(ask("comment je lance une lens dans Lens Studio quand le projet est fini"), None);
        assert_eq!(ask("lance"), None);
        assert_eq!(ask("ouvre moi"), None);
        assert_eq!(ask("joue avec moi"), None);
    }

    #[test]
    fn strips_politeness_around_commands() {
        assert_eq!(command_text("Pyrosan, monte le son stp !", "Pyrosan"), "monte le son");
        assert_eq!(command_text("hey pyrosan est-ce que tu peux couper le son", "Pyrosan"), "couper le son");
        assert_eq!(command_text("Musique suivante.", "Pyrosan"), "musique suivante");
    }

    #[test]
    fn finds_the_right_shortcut() {
        let dir = std::env::temp_dir().join(format!("claude-pet-launch-{}", crate::stats::now_ms()));
        let riot = dir.join("Riot Games");
        fs::create_dir_all(&riot).unwrap();
        for name in ["League of Legends.lnk", "League of Legends PBE.lnk", "Riot Client.lnk"] {
            fs::write(riot.join(name), "").unwrap();
        }
        for name in ["Discord.lnk", "Uninstall Discord.lnk", "Counter-Strike 2.url", "Spotify.lnk", "notes.txt", "Lens Studio.lnk"] {
            fs::write(dir.join(name), "").unwrap();
        }
        let dirs = [dir.clone()];
        let name = |q: &str| find(q, &dirs).map(|(n, _)| n);
        assert_eq!(name("league of legends").as_deref(), Some("League of Legends"));
        assert_eq!(name("lol").as_deref(), Some("League of Legends"));
        assert_eq!(name("league").as_deref(), Some("League of Legends"));
        assert_eq!(name("discord").as_deref(), Some("Discord"));
        assert_eq!(name("counter strike").as_deref(), Some("Counter-Strike 2"));
        assert_eq!(name("cs2").as_deref(), Some("Counter-Strike 2"));
        assert_eq!(name("lens studio").as_deref(), Some("Lens Studio"));
        assert_eq!(name("notes"), None, "only shortcuts count");
        assert_eq!(name("photoshop"), None);
        fs::remove_dir_all(dir).unwrap();
    }
}
