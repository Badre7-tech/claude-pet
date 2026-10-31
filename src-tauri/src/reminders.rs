//! "rappelle-moi dans 20 minutes de sortir le linge", "minuteur 10 min",
//! "tous les jours à 8h rappelle-moi de publier ma Lens". Understood locally, saved next to
//! the pet so they survive a restart, and announced out loud when they're due.

use crate::launcher::{normalize, PREAMBLE};
use chrono::{DateTime, Datelike, Days, Local, NaiveTime, TimeZone, Timelike};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::LazyLock,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reminder {
    /// When it goes off, in Unix seconds.
    pub due: i64,
    pub text: String,
    /// Comes back every day at the same time.
    #[serde(default)]
    pub daily: bool,
    #[serde(default)]
    pub timer: bool,
}

pub const MAX_REMINDERS: usize = 50;

#[derive(Default, Serialize, Deserialize)]
pub struct Reminders {
    items: Vec<Reminder>,
}

impl Reminders {
    fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("reminders.json")
    }

    pub fn load(data_dir: &Path) -> Self {
        crate::chat::load_json(&Self::path(data_dir))
    }

    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        crate::chat::save_json(&Self::path(data_dir), self)
    }

    #[allow(dead_code)]
    pub fn list(&self) -> &[Reminder] {
        &self.items
    }

    pub fn add(&mut self, reminder: Reminder) -> Result<(), String> {
        if self.items.len() >= MAX_REMINDERS {
            return Err(format!("j'ai déjà {MAX_REMINDERS} rappels, annules-en quelques-uns (/cancel)"));
        }
        self.items.push(reminder);
        self.items.sort_by_key(|r| r.due);
        Ok(())
    }

    /// Removes reminder number `n` (counting from 1, as /reminders shows them).
    pub fn remove(&mut self, n: usize) -> Option<Reminder> {
        (1..=self.items.len()).contains(&n).then(|| self.items.remove(n - 1))
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn clear_timers(&mut self) -> usize {
        let before = self.items.len();
        self.items.retain(|r| !r.timer);
        before - self.items.len()
    }

    /// Takes what's due: one-offs are removed, daily ones move to their next day.
    pub fn take_due(&mut self, now: i64) -> Vec<Reminder> {
        let fired: Vec<Reminder> = self.items.iter().filter(|r| r.due <= now).cloned().collect();
        for r in self.items.iter_mut().filter(|r| r.daily && r.due <= now) {
            r.due = next_daily(r.due, now);
        }
        self.items.retain(|r| r.daily || r.due > now);
        self.items.sort_by_key(|r| r.due);
        fired
    }

    /// After the app was closed for a while: a daily reminder missed long ago just waits for its
    /// next day, and a one-off more than half a day late is dropped (the rest still go off).
    pub fn catch_up(&mut self, now: i64) {
        for r in self.items.iter_mut().filter(|r| r.daily && r.due < now - 30 * 60) {
            r.due = next_daily(r.due, now);
        }
        self.items.retain(|r| r.daily || r.due > now - 12 * 3600);
        self.items.sort_by_key(|r| r.due);
    }

    /// "08:00 publier ma Lens" lines for what's left today, for the morning briefing.
    #[allow(dead_code)] // for the morning briefing
    pub fn today(&self, now: DateTime<Local>) -> Vec<String> {
        self.items
            .iter()
            .filter_map(|r| {
                let at = Local.timestamp_opt(r.due, 0).single()?;
                (at.date_naive() == now.date_naive() && r.due >= now.timestamp()).then(|| format!("{} {}", at.format("%H:%M"), r.text))
            })
            .collect()
    }

    pub fn describe(&self, now: DateTime<Local>) -> String {
        if self.items.is_empty() {
            return "aucun rappel en cours. essaie « rappelle-moi dans 20 minutes de sortir le linge »".into();
        }
        self.items
            .iter()
            .enumerate()
            .map(|(i, r)| format!("{}. {} — {}", i + 1, when(r.due, now, r.daily), r.text))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The same time of day as `due`, on the first day after `now`.
fn next_daily(due: i64, now: i64) -> i64 {
    let Some(at) = Local.timestamp_opt(due, 0).single() else { return due + 86_400 };
    let now_local = Local.timestamp_opt(now, 0).single().unwrap_or(at);
    let mut day = now_local.date_naive();
    loop {
        if let Some(next) = local_at(day, at.time()) {
            if next.timestamp() > now {
                return next.timestamp();
            }
        }
        day = day + Days::new(1);
    }
}

fn local_at(day: chrono::NaiveDate, time: NaiveTime) -> Option<DateTime<Local>> {
    Local.from_local_datetime(&day.and_time(time)).earliest()
}

/// "à 18h30", "demain à 09h00", "le 03/11 à 09h00", "tous les jours à 08h00".
fn when(due: i64, now: DateTime<Local>, daily: bool) -> String {
    let Some(at) = Local.timestamp_opt(due, 0).single() else { return "?".into() };
    let time = at.format("%Hh%M");
    if daily {
        return format!("tous les jours à {time}");
    }
    let days = (at.date_naive() - now.date_naive()).num_days();
    match days {
        0 => format!("à {time}"),
        1 => format!("demain à {time}"),
        _ => format!("le {} à {time}", at.format("%d/%m")),
    }
}

/// What the human asked for, if their message is about reminders or timers.
#[derive(Debug, PartialEq)]
pub enum Request {
    Add(Reminder),
    List,
    /// `None` cancels everything.
    Cancel(Option<usize>),
    CancelTimers,
}

const NUM: &str = r"(?:\d+(?:[.,]\d+)?|une?|deux|trois|quatre|cinq|six|sept|huit|neuf|dix|onze|douze|treize|quatorze|quinze|seize|dix-sept|dix-huit|dix-neuf|vingt|vingt-cinq|trente|quarante|quarante-cinq|cinquante|soixante)";

static DUR: LazyLock<String> = LazyLock::new(|| {
    format!(
        r"(?:{NUM}\s*(?:h|heures?)(?:\s*(?:et\s+)?(?:demie?|quart|\d{{1,2}})(?:\s*(?:minutes?|mins?|mn))?)?|{NUM}\s*(?:minutes?|mins?|mn|m)(?:\s*(?:et\s+)?\d{{1,2}}\s*(?:secondes?|s))?|{NUM}\s*(?:secondes?|secs?|s)|(?:un\s+)?quart\s+d'heure|(?:une\s+)?demi[- ]heure)"
    )
});
const TIME: &str = r"(?:midi|minuit|\d{1,2}\s*(?:h|heures?|:)\s*(?:\d{2})?)(?:\s*(?:du\s+(?:matin|soir)|de\s+l'apres-midi))?";
const DAY: &str = r"(?:demain|apres-demain|ce soir|ce matin|cet apres-midi|aujourd'hui|cette nuit)";
const DAILY: &str = r"tous les (?:jours|matins|soirs)";
const LINK: &str = r"(?:de |d'|qu'|que |pour |a propos de )?";

macro_rules! re {
    ($($arg:tt)*) => {
        LazyLock::new(|| Regex::new(&format!($($arg)*)).unwrap())
    };
}

static REMIND: LazyLock<Regex> = re!(r"^(?:rappelle|rapelle|rappel|rappelles|previens|previens)[- ](?:moi|nous)\s+(.+)$");
static TIMER: LazyLock<Regex> =
    re!(r"^(?:(?:mets|met|lance|programme|demarre)(?:[- ]moi)? )?(?:un )?(?:minuteur|timer|chrono|compte a rebours)(?: de| pour| sur)? ({})$", *DUR);
static IN: LazyLock<Regex> = re!(r"^dans ({})[ ,]*{LINK}(.*)$", *DUR);
static AT: LazyLock<Regex> = re!(r"^(?:({DAY}) )?(?:a |vers |pour )?({TIME})[ ,]*{LINK}(.*)$");
static EVERY: LazyLock<Regex> = re!(r"^{DAILY} (?:a |vers )?({TIME})[ ,]*(?:(?:rappelle|previens)[- ]moi )?{LINK}(.*)$");
static TEXT_IN: LazyLock<Regex> = re!(r"^{LINK}(.+?)[ ,]+dans ({})$", *DUR);
static TEXT_EVERY: LazyLock<Regex> = re!(r"^{LINK}(.+?)[ ,]+{DAILY} (?:a |vers )?({TIME})$");
static TEXT_AT: LazyLock<Regex> = re!(r"^{LINK}(.+?)[ ,]+(?:({DAY}) )?(?:a |vers )({TIME})$");
static LIST: LazyLock<Regex> = re!(r"^(?:(?:quels? sont|montre(?: moi)?|liste|affiche|donne(?: moi)?) )?(?:mes|les) (?:rappels|minuteurs|alarmes)(?: en cours)?$");
static CANCEL: LazyLock<Regex> = re!(
    r"^(?:annule|supprime|efface|enleve|stoppe|arrete)(?: moi)? (?:(tous|toutes) )?(?:le |la |les |mon |ma |mes )?(rappels?|minuteurs?|timers?|alarmes?)(?: (?:numero |n )?(\d+))?$"
);

/// Folds accents one character for one character, so positions match the original text.
fn fold(c: char) -> char {
    match c {
        'à' | 'â' | 'ä' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'î' | 'ï' => 'i',
        'ô' | 'ö' => 'o',
        'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        '’' => '\'',
        c => c,
    }
}

/// The original text for a byte range of the folded copy (both have the same characters).
fn original(orig: &str, folded: &str, range: std::ops::Range<usize>) -> String {
    let start = folded[..range.start].chars().count();
    let len = folded[range].chars().count();
    orig.chars().skip(start).take(len).collect()
}

fn number(word: &str) -> Option<f64> {
    if let Ok(n) = word.replace(',', ".").parse::<f64>() {
        return Some(n);
    }
    Some(match word {
        "un" | "une" => 1.0,
        "deux" => 2.0,
        "trois" => 3.0,
        "quatre" => 4.0,
        "cinq" => 5.0,
        "six" => 6.0,
        "sept" => 7.0,
        "huit" => 8.0,
        "neuf" => 9.0,
        "dix" => 10.0,
        "onze" => 11.0,
        "douze" => 12.0,
        "treize" => 13.0,
        "quatorze" => 14.0,
        "quinze" => 15.0,
        "seize" => 16.0,
        "dix-sept" => 17.0,
        "dix-huit" => 18.0,
        "dix-neuf" => 19.0,
        "vingt" => 20.0,
        "vingt-cinq" => 25.0,
        "trente" => 30.0,
        "quarante" => 40.0,
        "quarante-cinq" => 45.0,
        "cinquante" => 50.0,
        "soixante" => 60.0,
        _ => return None,
    })
}

/// "20 minutes", "1h30", "une heure et demie", "un quart d'heure", "30 s" → seconds.
pub fn duration_secs(text: &str) -> Option<i64> {
    let t = text.trim();
    if t.contains("quart d'heure") {
        return Some(15 * 60);
    }
    if t.contains("demi-heure") || t.contains("demi heure") {
        return Some(30 * 60);
    }
    static PARTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+(?:[.,]\d+)?|[a-z-]+").unwrap());
    let parts: Vec<&str> = PARTS.find_iter(t).map(|m| m.as_str()).filter(|p| *p != "et").collect();
    let value = number(parts.first()?)?;
    let unit = parts.get(1).copied().unwrap_or("min");
    let secs = if unit.starts_with('h') {
        let extra = match parts.get(2).copied() {
            Some("demi" | "demie") => 30.0,
            Some("quart") => 15.0,
            Some(n) => number(n).unwrap_or(0.0),
            None => 0.0,
        };
        value * 3600.0 + extra * 60.0
    } else if unit.starts_with('m') {
        let extra = parts.get(2).and_then(|n| number(n)).unwrap_or(0.0);
        value * 60.0 + extra
    } else {
        value
    };
    (secs >= 1.0 && secs <= 7.0 * 86_400.0).then_some(secs.round() as i64)
}

/// "18h30", "8 heures du soir", "midi", "9:05" → (hour, minute).
fn time_of_day(text: &str, evening: bool) -> Option<(u32, u32)> {
    let t = text.trim();
    let (mut h, m) = match t {
        _ if t.starts_with("midi") => (12, 0),
        _ if t.starts_with("minuit") => (0, 0),
        _ => {
            static HM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{1,2})\s*(?:h|heures?|:)\s*(\d{2})?").unwrap());
            let c = HM.captures(t)?;
            (c[1].parse().ok()?, c.get(2).map_or(Some(0), |m| m.as_str().parse().ok())?)
        }
    };
    if (evening || t.contains("soir") || t.contains("apres-midi")) && h < 12 {
        h += 12;
    }
    (h < 24 && m < 60).then_some((h, m))
}

fn at_time(day: Option<&str>, time: &str, now: DateTime<Local>) -> Result<i64, String> {
    let evening = matches!(day, Some("ce soir" | "cet apres-midi" | "cette nuit"));
    let (h, m) = time_of_day(time, evening).ok_or("je n'ai pas compris l'heure")?;
    let offset = match day {
        Some("demain") => 1,
        Some("apres-demain") => 2,
        _ => 0,
    };
    let date = now.date_naive() + Days::new(offset);
    let at = local_at(date, NaiveTime::from_hms_opt(h, m, 0).ok_or("heure invalide")?).ok_or("heure invalide")?;
    if at > now {
        return Ok(at.timestamp());
    }
    if day.is_some() {
        return Err("cette heure-là est déjà passée".into());
    }
    // "à 8h" said at 10h means tomorrow morning.
    local_at(date + Days::new(1), at.time()).map(|t| t.timestamp()).ok_or_else(|| "heure invalide".into())
}

/// "de sortir le linge" → "Sortir le linge".
fn tidy(text: &str) -> String {
    let t = text.trim().trim_matches(|c: char| c == ',' || c == '.' || c == '!' || c == '?' || c.is_whitespace());
    let mut chars = t.chars();
    match chars.next() {
        None => "c'est l'heure !".into(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

fn minutes(secs: i64) -> String {
    match secs {
        s if s < 60 => format!("{s} s"),
        s if s < 3600 => format!("{} min", (s + 30) / 60),
        s if s % 3600 == 0 => format!("{} h", s / 3600),
        s => format!("{} h {:02}", s / 3600, (s % 3600) / 60),
    }
}

/// Recognises reminder requests; `Some(Err(…))` means it was one but something didn't make sense.
pub fn parse(message: &str, pet_name: &str, now: DateTime<Local>) -> Option<Result<Request, String>> {
    let pet = normalize(pet_name);
    let text = message.replace('’', "'");
    // Drop "hey pyrosan," and the like in front, as for every other command.
    let mut rest = text.trim();
    while let Some((word, tail)) = rest.split_once(char::is_whitespace) {
        let n = normalize(word);
        if n.is_empty() || n.split(' ').all(|w| PREAMBLE.contains(&w) || w == pet) {
            rest = tail.trim_start();
        } else {
            break;
        }
    }
    let orig = rest.trim_end_matches(|c: char| ".!?".contains(c) || c.is_whitespace()).to_string();
    // Lowercased and accent-free, one character for one, so matches map back onto `orig`.
    let folded: String = orig.chars().map(|c| fold(c.to_lowercase().next().unwrap_or(c))).collect();
    let plain = normalize(&orig);

    if LIST.is_match(&plain) {
        return Some(Ok(Request::List));
    }
    if let Some(c) = CANCEL.captures(&plain) {
        let what = &c[2];
        if let Some(n) = c.get(3) {
            return Some(n.as_str().parse().map(|n| Request::Cancel(Some(n))).map_err(|_| "quel numéro ?".into()));
        }
        if what.starts_with("minuteur") || what.starts_with("timer") {
            return Some(Ok(Request::CancelTimers));
        }
        return Some(Ok(Request::Cancel(None)));
    }
    if let Some(c) = TIMER.captures(&folded) {
        let secs = duration_secs(&c[1])?;
        let text = format!("minuteur de {} terminé !", minutes(secs));
        return Some(Ok(Request::Add(Reminder { due: now.timestamp() + secs, text, daily: false, timer: true })));
    }
    let daily_first = EVERY.captures(&folded);
    let after = match (&daily_first, REMIND.captures(&folded)) {
        (Some(_), _) => None,
        (None, Some(c)) => Some(c.get(1).unwrap().range()),
        (None, None) => return None,
    };
    let text_of = |m: regex::Match| original(&orig, &folded, m.range());
    let add = |due: Result<i64, String>, text: String, daily: bool| {
        due.map(|due| Request::Add(Reminder { due, text: tidy(&text), daily, timer: false }))
    };
    if let Some(c) = daily_first {
        return Some(add(at_time(None, &c[1], now), text_of(c.get(2).unwrap()), true));
    }
    let start = after.unwrap().start;
    let body = &folded[start..];
    let shifted = |m: regex::Match| original(&orig, &folded, (m.start() + start)..(m.end() + start));
    let in_secs = |d: &str| duration_secs(d).map(|s| now.timestamp() + s).ok_or_else(|| "je n'ai pas compris dans combien de temps".to_string());

    Some(if let Some(c) = IN.captures(body) {
        add(in_secs(&c[1]), shifted(c.get(2).unwrap()), false)
    } else if let Some(c) = EVERY.captures(body) {
        add(at_time(None, &c[1], now), shifted(c.get(2).unwrap()), true)
    } else if let Some(c) = AT.captures(body) {
        add(at_time(c.get(1).map(|d| d.as_str()), &c[2], now), shifted(c.get(3).unwrap()), false)
    } else if let Some(c) = TEXT_IN.captures(body) {
        add(in_secs(&c[2]), shifted(c.get(1).unwrap()), false)
    } else if let Some(c) = TEXT_EVERY.captures(body) {
        add(at_time(None, &c[2], now), shifted(c.get(1).unwrap()), true)
    } else if let Some(c) = TEXT_AT.captures(body) {
        add(at_time(c.get(2).map(|d| d.as_str()), &c[3], now), shifted(c.get(1).unwrap()), false)
    } else {
        Err("dis-moi quand : « rappelle-moi dans 20 minutes de sortir le linge » ou « rappelle-moi à 18h d'appeler Karim »".into())
    })
}

/// The confirmation line for a reminder that was just set.
pub fn confirm(r: &Reminder, now: DateTime<Local>) -> String {
    if r.timer {
        return format!("⏱️ minuteur lancé : {}", minutes(r.due - now.timestamp()).replace(' ', " "));
    }
    let soon = r.due - now.timestamp();
    let when = if !r.daily && soon < 3 * 3600 {
        format!("dans {} ({})", minutes(soon), self::when(r.due, now, false))
    } else {
        self::when(r.due, now, r.daily)
    };
    format!("⏰ c'est noté : « {} », {when}", r.text)
}

/// What the pet says when a reminder goes off.
pub fn announce(r: &Reminder, now: i64) -> String {
    let late = now - r.due > 120;
    match (r.timer, late) {
        (true, _) => format!("⏱️ {}", r.text),
        (false, false) => format!("⏰ c'est l'heure : {}", r.text),
        (false, true) => format!("⏰ (en retard, j'étais éteint) {}", r.text),
    }
}

#[allow(dead_code)]
fn weekday_fr(d: DateTime<Local>) -> &'static str {
    ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"][d.weekday().num_days_from_monday() as usize]
}

/// "samedi 31 octobre 2026, 10:15": the pet's sense of time, sent with every message.
pub fn local_now_text(now: DateTime<Local>) -> String {
    const MONTHS: [&str; 12] =
        ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"];
    format!("{} {} {} {}, {:02}:{:02}", weekday_fr(now), now.day(), MONTHS[now.month0() as usize], now.year(), now.hour(), now.minute())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 31, 10, 0, 0).unwrap()
    }

    fn add(m: &str) -> Reminder {
        match parse(m, "Pyrosan", now()) {
            Some(Ok(Request::Add(r))) => r,
            other => panic!("{m:?} gave {other:?}"),
        }
    }

    fn at(h: u32, m: u32, days: u64) -> i64 {
        local_at(now().date_naive() + Days::new(days), NaiveTime::from_hms_opt(h, m, 0).unwrap()).unwrap().timestamp()
    }

    #[test]
    fn durations() {
        assert_eq!(duration_secs("20 minutes"), Some(1200));
        assert_eq!(duration_secs("1h30"), Some(5400));
        assert_eq!(duration_secs("une heure et demie"), Some(5400));
        assert_eq!(duration_secs("un quart d'heure"), Some(900));
        assert_eq!(duration_secs("une demi-heure"), Some(1800));
        assert_eq!(duration_secs("30 secondes"), Some(30));
        assert_eq!(duration_secs("2 h"), Some(7200));
        assert_eq!(duration_secs("dix min"), Some(600));
    }

    #[test]
    fn reminders_in_and_at() {
        let r = add("Rappelle-moi dans 20 minutes de sortir le linge.");
        assert_eq!((r.due, r.text.as_str(), r.daily), (now().timestamp() + 1200, "Sortir le linge", false));
        let r = add("hey pyrosan rappelle moi dans 1h30 qu'il faut appeler l'école");
        assert_eq!((r.due, r.text.as_str()), (now().timestamp() + 5400, "Il faut appeler l'école"));
        let r = add("rappelle-moi à 18h30 d'appeler Karim");
        assert_eq!((r.due, r.text.as_str()), (at(18, 30, 0), "Appeler Karim"));
        let r = add("rappelle-moi demain à 9h de publier la lens");
        assert_eq!((r.due, r.text.as_str()), (at(9, 0, 1), "Publier la lens"));
        let r = add("rappelle-moi à 8h de faire du sport");
        assert_eq!(r.due, at(8, 0, 1), "8h already passed today → tomorrow");
        let r = add("rappelle-moi ce soir à 8h de lancer une game");
        assert_eq!(r.due, at(20, 0, 0));
        let r = add("rappelle-moi de sortir les poubelles dans 10 minutes");
        assert_eq!((r.due, r.text.as_str()), (now().timestamp() + 600, "Sortir les poubelles"));
        let r = add("rappelle-moi de prendre mes clés à 14h");
        assert_eq!((r.due, r.text.as_str()), (at(14, 0, 0), "Prendre mes clés"));
    }

    #[test]
    fn daily_and_timers() {
        let r = add("tous les jours à 8h rappelle-moi de publier ma Lens");
        assert_eq!((r.due, r.text.as_str(), r.daily), (at(8, 0, 1), "Publier ma Lens", true));
        let r = add("rappelle-moi tous les jours à 12h30 de boire de l'eau");
        assert_eq!((r.due, r.text.as_str(), r.daily), (at(12, 30, 0), "Boire de l'eau", true));
        let r = add("mets un minuteur de 10 minutes");
        assert!(r.timer);
        assert_eq!(r.due, now().timestamp() + 600);
        let r = add("minuteur 5 min");
        assert_eq!(r.due, now().timestamp() + 300);
    }

    #[test]
    fn list_cancel_and_nonsense() {
        let p = |m: &str| parse(m, "Pyrosan", now());
        assert_eq!(p("quels sont mes rappels ?"), Some(Ok(Request::List)));
        assert_eq!(p("annule le rappel 2"), Some(Ok(Request::Cancel(Some(2)))));
        assert_eq!(p("annule tous les rappels"), Some(Ok(Request::Cancel(None))));
        assert_eq!(p("annule le minuteur"), Some(Ok(Request::CancelTimers)));
        assert!(matches!(p("rappelle-moi de manger"), Some(Err(_))), "no time given");
        assert_eq!(p("tu te rappelles de mon rang ?"), None);
        assert_eq!(p("quel temps demain ?"), None);
    }

    #[test]
    fn due_reminders_fire_and_daily_ones_come_back() {
        let mut list = Reminders::default();
        let t = now().timestamp();
        list.add(Reminder { due: t + 60, text: "once".into(), daily: false, timer: false }).unwrap();
        list.add(Reminder { due: t + 120, text: "daily".into(), daily: true, timer: false }).unwrap();
        assert!(list.take_due(t).is_empty());
        let fired = list.take_due(t + 200);
        assert_eq!(fired.len(), 2);
        assert_eq!(list.list().len(), 1);
        assert_eq!(list.list()[0].due, t + 120 + 86_400);
        assert_eq!(list.remove(1).unwrap().text, "daily");
        assert!(list.remove(1).is_none());
    }

    #[test]
    fn catching_up_after_being_closed() {
        let mut list = Reminders::default();
        let t = now().timestamp();
        list.add(Reminder { due: t - 3600, text: "missed".into(), daily: false, timer: false }).unwrap();
        list.add(Reminder { due: t - 20 * 3600, text: "ancient".into(), daily: false, timer: false }).unwrap();
        list.add(Reminder { due: t - 5 * 3600, text: "daily".into(), daily: true, timer: false }).unwrap();
        list.catch_up(t);
        let texts: Vec<&str> = list.list().iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, ["missed", "daily"]);
        assert_eq!(list.list()[1].due, t - 5 * 3600 + 86_400);
        assert!(announce(&list.list()[0], t).contains("en retard"));
    }

    #[test]
    fn time_text_is_french() {
        assert_eq!(local_now_text(now()), "samedi 31 octobre 2026, 10:00");
    }
}
