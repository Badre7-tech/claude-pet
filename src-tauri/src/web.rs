//! "mets YouTube", "mets une vidéo de Squeezie", "cherche des recettes sur YouTube":
//! opens a well-known site, or its search page, in the browser. A fixed list of sites, and
//! the human's own words only go into the search box.

use crate::launcher::command_text;
use regex::Regex;
use std::sync::LazyLock;

/// Sites the pet knows: (what people say, home page, search page with `{}` for the words).
const SITES: &[(&str, &str, &str)] = &[
    ("youtube", "https://www.youtube.com", "https://www.youtube.com/results?search_query={}"),
    ("twitch", "https://www.twitch.tv", "https://www.twitch.tv/search?term={}"),
    ("netflix", "https://www.netflix.com", "https://www.netflix.com/search?q={}"),
    ("google", "https://www.google.com", "https://www.google.com/search?q={}"),
    ("gmail", "https://mail.google.com", "https://mail.google.com/mail/u/0/#search/{}"),
    ("instagram", "https://www.instagram.com", "https://www.instagram.com/explore/search/keyword/?q={}"),
    ("tiktok", "https://www.tiktok.com", "https://www.tiktok.com/search?q={}"),
    ("snapchat", "https://www.snapchat.com", "https://www.snapchat.com/explore/{}"),
    ("twitter", "https://x.com", "https://x.com/search?q={}"),
    ("reddit", "https://www.reddit.com", "https://www.reddit.com/search/?q={}"),
    ("op gg", "https://www.op.gg", "https://www.op.gg/search?q={}"),
    ("opgg", "https://www.op.gg", "https://www.op.gg/search?q={}"),
    ("amazon", "https://www.amazon.fr", "https://www.amazon.fr/s?k={}"),
    ("leboncoin", "https://www.leboncoin.fr", "https://www.leboncoin.fr/recherche?text={}"),
];

const OPEN: &str = r"(?:mets|met|mettre|metz|lance|lancer|ouvre|ouvrir|affiche|afficher|va sur|vas sur|aller sur)(?: moi)?";
const SITE: &str = r"(youtube|twitch|netflix|google|gmail|instagram|tiktok|snapchat|twitter|reddit|op gg|opgg|amazon|leboncoin)";

static HOME: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"^{OPEN} (?:le site |le |la )?{SITE}$")).unwrap());
static SEARCH_ON: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"^(?:{OPEN}|cherche|cherches|chercher|recherche|trouve)(?: moi)? (.+?) (?:sur|dans) {SITE}$")).unwrap()
});
/// "mets une vidéo de Squeezie": YouTube unless another site is named.
static VIDEO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^{OPEN} (?:une |des |la |le |un |de la |du |de l |de )?(?:video|videos|clip|clips|musique|musiques|chanson|chansons|son|sons|zik) (?:de |d |du |des |sur |avec )?(.+)$"
    ))
    .unwrap()
});

/// "des clips de", "une vidéo de"… in front of what to search for.
static KIND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:une |des |la |le |un |de la |du |de l |de )?(?:video|videos|clip|clips|musique|musiques|chanson|chansons|son|sons|zik) (?:de |d |du |des |sur |avec )?").unwrap()
});

fn site(name: &str) -> Option<&'static (&'static str, &'static str, &'static str)> {
    SITES.iter().find(|s| s.0 == name)
}

/// Letters and digits stay, spaces become +, everything else is %-encoded.
fn encode(words: &str) -> String {
    words
        .bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The page to open and what to say, if the message asks for a site or a search on one.
pub fn parse(message: &str, pet_name: &str) -> Option<(String, String)> {
    let text = command_text(message, pet_name);
    if let Some(c) = SEARCH_ON.captures(&text) {
        let query = KIND.replace(c[1].trim(), "");
        let (query, (label, _, search)) = (query.trim(), site(&c[2])?);
        return Some((search.replace("{}", &encode(query)), format!("▶️ je cherche « {query} » sur {}", title(label))));
    }
    if let Some(c) = VIDEO.captures(&text) {
        let query = c[1].trim().trim_end_matches(" sur youtube");
        return Some((SITES[0].2.replace("{}", &encode(query)), format!("▶️ je mets « {query} » sur YouTube")));
    }
    if let Some(c) = HOME.captures(&text) {
        let (label, home, _) = site(&c[1])?;
        return Some((home.to_string(), format!("▶️ j'ouvre {}", title(label))));
    }
    None
}

fn title(label: &str) -> String {
    match label {
        "youtube" => "YouTube".into(),
        "tiktok" => "TikTok".into(),
        "op gg" | "opgg" => "op.gg".into(),
        "leboncoin" => "leboncoin".into(),
        other => {
            let mut c = other.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(m: &str) -> Option<String> {
        parse(m, "Pyrosan").map(|(url, _)| url)
    }

    #[test]
    fn opens_sites_and_searches() {
        assert_eq!(p("mets YouTube").as_deref(), Some("https://www.youtube.com"));
        assert_eq!(p("Tu peux mettre YouTube s'te plaît").as_deref(), Some("https://www.youtube.com"));
        assert_eq!(p("Tu mets une vidéo de Squeezie.").as_deref(), Some("https://www.youtube.com/results?search_query=squeezie"));
        assert_eq!(p("mets de la musique de Ninho").as_deref(), Some("https://www.youtube.com/results?search_query=ninho"));
        assert_eq!(p("mets des clips de Ninho sur YouTube").as_deref(), Some("https://www.youtube.com/results?search_query=ninho"));
        assert_eq!(p("cherche recette tajine sur YouTube").as_deref(), Some("https://www.youtube.com/results?search_query=recette+tajine"));
        assert_eq!(p("ouvre Twitch").as_deref(), Some("https://www.twitch.tv"));
        assert_eq!(p("cherche Kameto sur Twitch").as_deref(), Some("https://www.twitch.tv/search?term=kameto"));
        assert_eq!(p("quel temps fait-il ?"), None);
        assert_eq!(p("lance League of Legends"), None);
    }
}
