//! The pet's ElevenLabs voice. Every sentence it has already said is kept on disk and replayed
//! for free, so only new answers spend ElevenLabs credits. Without a key (or when ElevenLabs
//! says no) the window falls back to the Windows voice.

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

/// Stops the cache from growing forever: oldest files go first past this many.
const MAX_CACHED: usize = 2000;

/// FNV-1a: a stable name for a cached clip (the same text, voice and model → the same file).
fn fingerprint(parts: &[&str]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in part.bytes().chain([0xff]) {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

pub fn model_id(choice: &str) -> &'static str {
    match choice {
        "quality" => "eleven_multilingual_v2",
        _ => "eleven_flash_v2_5",
    }
}

pub fn valid_key(key: &str) -> bool {
    (10..=200).contains(&key.len()) && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn valid_voice(voice: &str) -> bool {
    (10..=64).contains(&voice.len()) && voice.chars().all(|c| c.is_ascii_alphanumeric())
}

fn cache_file(dir: &Path, voice: &str, model: &str, text: &str) -> PathBuf {
    dir.join(format!("{}.mp3", fingerprint(&[voice, model, text.trim()])))
}

/// Keeps the newest `MAX_CACHED` clips.
fn prune(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if files.len() <= MAX_CACHED {
        return;
    }
    files.sort();
    for (_, path) in &files[..files.len() - MAX_CACHED] {
        let _ = fs::remove_file(path);
    }
}

/// The MP3 for `text`: from the cache when it was said before, otherwise from ElevenLabs.
pub fn speak(cache: &Path, key: &str, voice: &str, model: &str, text: &str) -> Result<Vec<u8>, String> {
    if key.is_empty() {
        return Err("not_configured".into());
    }
    let file = cache_file(cache, voice, model, text);
    if let Ok(bytes) = fs::read(&file) {
        return Ok(bytes);
    }
    let bytes = fetch(key, voice, model, text)?;
    if fs::create_dir_all(cache).is_ok() && fs::write(&file, &bytes).is_ok() {
        prune(cache);
    }
    Ok(bytes)
}

fn fetch(key: &str, voice: &str, model: &str, text: &str) -> Result<Vec<u8>, String> {
    let tls = native_tls::TlsConnector::new().map_err(|e| format!("ElevenLabs : {e}"))?;
    let agent = ureq::AgentBuilder::new().tls_connector(Arc::new(tls)).timeout(Duration::from_secs(40)).build();
    let url = format!("https://api.elevenlabs.io/v1/text-to-speech/{voice}?output_format=mp3_44100_128");
    let body = serde_json::json!({ "text": text, "model_id": model });
    match agent.post(&url).set("xi-api-key", key).set("accept", "audio/mpeg").send_json(body) {
        Ok(response) => {
            let mut bytes = Vec::new();
            response.into_reader().take(20_000_000).read_to_end(&mut bytes).map_err(|e| format!("ElevenLabs : {e}"))?;
            Ok(bytes)
        }
        Err(ureq::Error::Status(code, response)) => {
            let detail = response.into_string().unwrap_or_default().to_lowercase();
            Err(match code {
                401 => "ElevenLabs refuse la clé (vérifie-la avec /settings elevenkey)".into(),
                402 => "ElevenLabs : cette voix demande un abonnement payant".into(),
                404 => "ElevenLabs ne trouve pas cette voix (vérifie /settings elevenvoice)".into(),
                429 if detail.contains("quota") => "plus de crédits ElevenLabs ce mois-ci".into(),
                _ if detail.contains("quota") => "plus de crédits ElevenLabs ce mois-ci".into(),
                429 => "ElevenLabs est saturé, réessaie dans un instant".into(),
                _ => format!("ElevenLabs a répondu {code}"),
            })
        }
        Err(_) => Err("pas de connexion à ElevenLabs".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_clips_are_reused_without_a_request() {
        let dir = std::env::temp_dir().join(format!("claude-pet-tts-{}", crate::stats::now_ms()));
        let file = cache_file(&dir, "voice123456", "eleven_flash_v2_5", "salut le B");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&file, b"mp3!").unwrap();
        // A fake key would fail on the network, so a hit proves the cache answered.
        assert_eq!(speak(&dir, "fake_key_123", "voice123456", "eleven_flash_v2_5", " salut le B ").unwrap(), b"mp3!");
        assert_ne!(cache_file(&dir, "voice123456", "eleven_multilingual_v2", "salut le B"), file, "another model, another clip");
        assert_eq!(speak(&dir, "", "voice123456", "eleven_flash_v2_5", "salut").unwrap_err(), "not_configured");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn keys_and_voices_are_checked() {
        assert!(valid_voice("B8gJV1IhpuegLxdpXFOE"));
        assert!(!valid_voice("../../etc"));
        assert!(valid_key("sk_0123456789abcdef"));
        assert!(!valid_key("sk 123"));
        assert_eq!(model_id("quality"), "eleven_multilingual_v2");
        assert_eq!(model_id("anything"), "eleven_flash_v2_5");
    }
}
