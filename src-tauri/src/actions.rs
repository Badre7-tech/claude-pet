//! A short, fixed list of things the pet can do to the computer when the human asks:
//! volume, music, closing an app, locking or putting the PC to sleep. Like opening apps,
//! these are recognised in the human's own message and never come from Claude's reply.

use crate::launcher::command_text;
use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, PartialEq)]
pub enum Action {
    VolumeUp,
    VolumeDown,
    VolumeSet(u8),
    Mute,
    Unmute,
    PlayPause,
    Next,
    Previous,
    Close(String),
    Lock,
    Sleep,
}

/// How much "monte le son" / "baisse le son" changes the volume, in percent.
const VOLUME_STEP: i32 = 10;

static VOLUME_SET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:(?:mets|met|regle|monte|baisse) )?(?:le |la )?(?:son|volume) (?:a |au |sur |de )?(\d{1,3})(?: pour ?cent| pourcents?)?$").unwrap()
});
static VOLUME_UP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:(?:monte|montes|monter|augmente|augmenter)(?: un peu)?(?: le| la)? (?:son|volume)(?: un peu)?|plus fort|monte|son plus fort)$").unwrap()
});
static VOLUME_DOWN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:(?:baisse|baisses|baisser|diminue|diminuer)(?: un peu)?(?: le| la)? (?:son|volume)(?: un peu)?|moins fort|baisse|son moins fort)$").unwrap()
});
static MUTE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:(?:coupe|couper|mute|desactive)(?: le| la)? (?:son|volume)|mute|silence|chut)$").unwrap()
});
static UNMUTE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:(?:remets|remet|remettre|reactive|rallume)(?: le| la)? (?:son|volume)|unmute)$").unwrap()
});
static PLAY_PAUSE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:(?:mets |met )?(?:pause|play)(?: la musique| la video| le son| la chanson)?|(?:pause|stop|stoppe|arrete|reprends|reprend|relance|remets|remet|lance|mets|met|coupe)(?: la| le)? (?:musique|video|chanson|zic|son de la musique))$").unwrap()
});
static NEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:(?:mets |met |passe a )?(?:la )?(?:musique|chanson|piste|titre|son|video) suivante?|suivant|suivante|next|skip|passe (?:la |le )?(?:musique|chanson|piste|titre|son)|change de (?:musique|chanson))$").unwrap()
});
static PREVIOUS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:(?:mets |met |remets |reviens a )?(?:la )?(?:musique|chanson|piste|titre|son|video) precedente?|precedent|precedente|previous|reviens en arriere)$").unwrap()
});
static CLOSE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:ferme|fermes|fermer|quitte|quitter|close)(?: moi)?(?: le| la| les| l| mon| ma| mes)?(?: jeu| appli| application| logiciel| fenetre)? (.+)$").unwrap()
});
static LOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:verrouille|verrouiller|verouille|lock)(?: le| la| l| ma| mon)?(?: pc| ordi| ordinateur| session| ecran)?$").unwrap()
});
static SLEEP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:(?:mets|met|passe|mettre)(?: le| l| mon)? (?:pc|ordi|ordinateur) en veille|mise en veille|mode veille|veille)$").unwrap()
});

/// What the human asked for, if their message is one of the known computer commands.
pub fn parse(message: &str, pet_name: &str) -> Option<Action> {
    let text = command_text(message, pet_name);
    if text.is_empty() || text.split(' ').count() > 8 {
        return None;
    }
    if let Some(c) = VOLUME_SET.captures(&text) {
        let level: u32 = c[1].parse().ok()?;
        return Some(Action::VolumeSet(level.min(100) as u8));
    }
    let checks: [(&LazyLock<Regex>, fn() -> Action); 9] = [
        (&UNMUTE, || Action::Unmute),
        (&MUTE, || Action::Mute),
        (&VOLUME_UP, || Action::VolumeUp),
        (&VOLUME_DOWN, || Action::VolumeDown),
        (&NEXT, || Action::Next),
        (&PREVIOUS, || Action::Previous),
        (&PLAY_PAUSE, || Action::PlayPause),
        (&LOCK, || Action::Lock),
        (&SLEEP, || Action::Sleep),
    ];
    if let Some((_, make)) = checks.iter().find(|(re, _)| re.is_match(&text)) {
        return Some(make());
    }
    let target = CLOSE.captures(&text)?[1].trim().to_string();
    // "ferme le PC" isn't closing an app, and shutting down isn't on the list.
    if ["pc", "ordi", "ordinateur", "session", "toi", "ta gueule"].contains(&target.as_str()) {
        return None;
    }
    Some(Action::Close(target))
}

/// Does it, and returns what to tell the human.
pub fn run(action: Action) -> Result<String, String> {
    // COM (used for the volume) wants a thread of its own, not one of the app's.
    std::thread::spawn(move || run_here(action)).join().map_err(|_| "oups, ça a planté".to_string())?
}

fn run_here(action: Action) -> Result<String, String> {
    match action {
        Action::VolumeUp | Action::VolumeDown => {
            let now = os::volume()?;
            let step = if action == Action::VolumeUp { VOLUME_STEP } else { -VOLUME_STEP };
            let level = (now as i32 + step).clamp(0, 100) as u8;
            os::set_volume(level)?;
            Ok(format!("🔊 volume à {level} %"))
        }
        Action::VolumeSet(level) => {
            os::set_volume(level)?;
            Ok(format!("🔊 volume à {level} %"))
        }
        Action::Mute => os::set_mute(true).map(|_| "🔇 son coupé".into()),
        Action::Unmute => os::set_mute(false).map(|_| "🔊 son remis".into()),
        Action::PlayPause => os::media(os::Media::PlayPause).map(|_| "⏯️ c'est fait".into()),
        Action::Next => os::media(os::Media::Next).map(|_| "⏭️ suivante !".into()),
        Action::Previous => os::media(os::Media::Previous).map(|_| "⏮️ précédente !".into()),
        Action::Close(name) => match os::close(&name)? {
            Some(closed) => Ok(format!("j'ai fermé {closed}")),
            None => Err(format!("je ne trouve pas de fenêtre « {name} » ouverte")),
        },
        Action::Lock => os::lock().map(|_| "🔒 PC verrouillé".into()),
        Action::Sleep => {
            // A moment to read (or hear) the goodbye before the screen goes dark.
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_secs(4));
                let _ = os::sleep();
            });
            Ok("bonne nuit ! je mets le PC en veille dans quelques secondes 💤".into())
        }
    }
}

/// Opens Windows voice typing (Win + H) in whatever text box has the focus.
pub fn start_dictation() -> Result<(), String> {
    os::dictate()
}

#[cfg(not(windows))]
mod os {
    const ONLY_WINDOWS: &str = "ça, je ne sais le faire que sous Windows";
    pub fn dictate() -> Result<(), String> {
        Err(ONLY_WINDOWS.into())
    }
    pub enum Media {
        PlayPause,
        Next,
        Previous,
    }
    pub fn volume() -> Result<u8, String> {
        Err(ONLY_WINDOWS.into())
    }
    pub fn set_volume(_: u8) -> Result<(), String> {
        Err(ONLY_WINDOWS.into())
    }
    pub fn set_mute(_: bool) -> Result<(), String> {
        Err(ONLY_WINDOWS.into())
    }
    pub fn media(_: Media) -> Result<(), String> {
        Err(ONLY_WINDOWS.into())
    }
    pub fn close(_: &str) -> Result<Option<String>, String> {
        Err(ONLY_WINDOWS.into())
    }
    pub fn lock() -> Result<(), String> {
        Err(ONLY_WINDOWS.into())
    }
    pub fn sleep() -> Result<(), String> {
        Err(ONLY_WINDOWS.into())
    }
}

#[cfg(windows)]
mod os {
    use crate::launcher::score;
    use windows::{
        core::PWSTR,
        Win32::{
            Foundation::{CloseHandle, HWND, LPARAM, WPARAM},
            Media::Audio::{eConsole, eRender, Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator, MMDeviceEnumerator},
            System::{
                Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED},
                Power::SetSuspendState,
                Shutdown::LockWorkStation,
                Threading::{GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION},
            },
            UI::{
                Input::KeyboardAndMouse::{
                    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
                    VK_H, VK_LWIN, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK,
                },
                WindowsAndMessaging::{
                    EnumWindows, GetClassNameW, GetWindow, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW,
                    GW_OWNER, WM_CLOSE,
                },
            },
        },
    };

    pub enum Media {
        PlayPause,
        Next,
        Previous,
    }

    fn endpoint() -> Result<IAudioEndpointVolume, String> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let devices: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| format!("pas d'accès au son ({e})"))?;
            let speakers = devices.GetDefaultAudioEndpoint(eRender, eConsole).map_err(|_| "aucune sortie son trouvée".to_string())?;
            speakers.Activate(CLSCTX_ALL, None).map_err(|e| format!("pas d'accès au volume ({e})"))
        }
    }

    pub fn volume() -> Result<u8, String> {
        let level = unsafe { endpoint()?.GetMasterVolumeLevelScalar() }.map_err(|e| e.to_string())?;
        Ok((level * 100.0).round() as u8)
    }

    pub fn set_volume(level: u8) -> Result<(), String> {
        let ep = endpoint()?;
        unsafe {
            // Changing the volume is usually meant to hear something, so unmute too.
            let _ = ep.SetMute(false, std::ptr::null());
            ep.SetMasterVolumeLevelScalar(level as f32 / 100.0, std::ptr::null()).map_err(|e| e.to_string())
        }
    }

    pub fn set_mute(mute: bool) -> Result<(), String> {
        unsafe { endpoint()?.SetMute(mute, std::ptr::null()) }.map_err(|e| e.to_string())
    }

    fn key(vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        }
    }

    fn send(inputs: &[INPUT]) -> Result<(), String> {
        let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent as usize == inputs.len() {
            Ok(())
        } else {
            Err("la touche n'est pas passée".into())
        }
    }

    pub fn media(media: Media) -> Result<(), String> {
        let vk = match media {
            Media::PlayPause => VK_MEDIA_PLAY_PAUSE,
            Media::Next => VK_MEDIA_NEXT_TRACK,
            Media::Previous => VK_MEDIA_PREV_TRACK,
        };
        send(&[key(vk, KEYBD_EVENT_FLAGS(0)), key(vk, KEYEVENTF_KEYUP)])
    }

    pub fn dictate() -> Result<(), String> {
        let down = KEYBD_EVENT_FLAGS(0);
        send(&[key(VK_LWIN, down), key(VK_H, down), key(VK_H, KEYEVENTF_KEYUP), key(VK_LWIN, KEYEVENTF_KEYUP)])
    }

    fn wide(buf: &[u16]) -> String {
        String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(buf.len())])
    }

    /// The program's file name without ".exe", e.g. "Discord".
    fn program_name(pid: u32) -> Option<String> {
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
            let _ = CloseHandle(process);
            if !ok {
                return None;
            }
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            let file = path.rsplit('\\').next()?.to_string();
            Some(file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(&file).to_string())
        }
    }

    struct Window {
        hwnd: HWND,
        title: String,
        program: String,
    }

    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
        let found = &mut *(lparam.0 as *mut Vec<Window>);
        let owned = GetWindow(hwnd, GW_OWNER).map(|o| !o.is_invalid()).unwrap_or(false);
        if !IsWindowVisible(hwnd).as_bool() || owned {
            return true.into();
        }
        let mut class = [0u16; 128];
        GetClassNameW(hwnd, &mut class);
        // The desktop and taskbar: "closing" them would bring up the shutdown dialog.
        if ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"].contains(&wide(&class).as_str()) {
            return true.into();
        }
        let mut title = [0u16; 512];
        if GetWindowTextW(hwnd, &mut title) == 0 {
            return true.into();
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == GetCurrentProcessId() {
            return true.into();
        }
        let program = program_name(pid).unwrap_or_default();
        found.push(Window { hwnd, title: wide(&title), program });
        true.into()
    }

    /// Asks the best-matching app's windows to close (like clicking ✕, so it can still ask to
    /// save). Returns the name of what was closed, or None if nothing matched.
    pub fn close(wanted: &str) -> Result<Option<String>, String> {
        let mut windows: Vec<Window> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(collect), LPARAM(&mut windows as *mut _ as isize));
        }
        let scored = |w: &Window| score(&w.program, wanted).max(score(&w.title, wanted));
        let Some(best) = windows.iter().max_by_key(|w| scored(w)).filter(|w| scored(w) >= 60) else {
            return Ok(None);
        };
        let program = best.program.clone();
        let by_program = score(&program, wanted) >= 60;
        let name = if by_program { program.clone() } else { best.title.clone() };
        for w in windows.iter().filter(|w| if by_program { w.program == program } else { w.hwnd == best.hwnd }) {
            unsafe {
                let _ = PostMessageW(Some(w.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        Ok(Some(name))
    }

    pub fn lock() -> Result<(), String> {
        unsafe { LockWorkStation() }.map_err(|e| e.to_string())
    }

    pub fn sleep() -> Result<(), String> {
        if unsafe { SetSuspendState(false, false, false) } {
            Ok(())
        } else {
            Err("la mise en veille a été refusée".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(m: &str) -> Option<Action> {
        parse(m, "Pyrosan")
    }

    #[test]
    fn understands_volume_and_music() {
        assert_eq!(p("monte le son"), Some(Action::VolumeUp));
        assert_eq!(p("Pyrosan, monte un peu le volume stp"), Some(Action::VolumeUp));
        assert_eq!(p("baisse le son"), Some(Action::VolumeDown));
        assert_eq!(p("moins fort"), Some(Action::VolumeDown));
        assert_eq!(p("mets le son à 30 %"), Some(Action::VolumeSet(30)));
        assert_eq!(p("volume 75"), Some(Action::VolumeSet(75)));
        assert_eq!(p("coupe le son"), Some(Action::Mute));
        assert_eq!(p("remets le son"), Some(Action::Unmute));
        assert_eq!(p("pause"), Some(Action::PlayPause));
        assert_eq!(p("mets pause"), Some(Action::PlayPause));
        assert_eq!(p("Pause la musique."), Some(Action::PlayPause));
        assert_eq!(p("reprends la musique"), Some(Action::PlayPause));
        assert_eq!(p("musique suivante"), Some(Action::Next));
        assert_eq!(p("passe la chanson"), Some(Action::Next));
        assert_eq!(p("chanson précédente"), Some(Action::Previous));
    }

    #[test]
    fn understands_apps_and_power() {
        assert_eq!(p("ferme Discord"), Some(Action::Close("discord".into())));
        assert_eq!(p("hey pyrosan ferme moi le jeu league of legends"), Some(Action::Close("league of legends".into())));
        assert_eq!(p("quitte spotify stp"), Some(Action::Close("spotify".into())));
        assert_eq!(p("ferme le pc"), None, "never shuts the computer down");
        assert_eq!(p("verrouille le PC"), Some(Action::Lock));
        assert_eq!(p("mets le PC en veille"), Some(Action::Sleep));
        assert_eq!(p("mets l'ordi en veille"), Some(Action::Sleep));
    }

    #[test]
    fn leaves_conversation_alone() {
        assert_eq!(p("le son de ce jeu est trop bien"), None);
        assert_eq!(p("j'ai fermé la porte"), None);
        assert_eq!(p("quel est le meilleur volume pour dormir ?"), None);
        assert_eq!(p("tu peux me conseiller une musique ?"), None);
        assert_eq!(p("lance league of legends"), None, "opening apps is the launcher's job");
    }
}
