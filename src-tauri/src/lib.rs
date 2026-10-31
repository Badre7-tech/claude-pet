mod actions;
mod chat;
mod launcher;
mod personality;
mod reminders;
mod screen;
mod settings;
mod stats;
mod tray;
mod tts;
mod voice_in;

use chat::{History, Notes, RateLimiter};
use settings::{Command, Settings};
use stats::{now_ms, Outcome, Pet, Store};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State, WebviewWindow};

const TICK: Duration = Duration::from_secs(5);

pub struct AppState {
    pet: Mutex<Pet>,
    store: Store,
    settings: Mutex<Settings>,
    data_dir: PathBuf,
    limiter: Mutex<RateLimiter>,
    history: Mutex<History>,
    notes: Mutex<Notes>,
    reminders: Mutex<reminders::Reminders>,
    thinking: AtomicBool,
    /// The day the morning briefing was given, so it only happens once.
    briefed: Mutex<Option<chrono::NaiveDate>>,
}

impl AppState {
    /// Folder whose CLAUDE.md is the current skin's personality (created on first use).
    pub fn brain(&self) -> PathBuf {
        let skin = self.settings.lock().unwrap().skin.clone();
        chat::brain_dir(&self.data_dir, &skin).unwrap_or_else(|_| self.data_dir.join("brain"))
    }

    pub fn skin(&self) -> String {
        self.settings.lock().unwrap().skin.clone()
    }

    /// Changes the conversation history and saves it, so it survives a restart.
    fn update_history(&self, f: impl FnOnce(&mut History)) {
        let mut history = self.history.lock().unwrap();
        f(&mut history);
        if let Err(e) = history.save(&self.data_dir) {
            eprintln!("failed to save history: {e}");
        }
    }
}

/// Clears the "thinking" flag however the chat call ends.
struct Thinking<'a>(&'a AtomicBool);

impl Drop for Thinking<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Catches the pet up on decay, applies `f`, then saves and broadcasts the result.
/// Every stat change goes through here so the UI, tray and save file never disagree.
pub fn with_pet<R>(app: &AppHandle, f: impl FnOnce(&mut Pet) -> R) -> R {
    let state = app.state::<AppState>();
    let (speed, name) = {
        let s = state.settings.lock().unwrap();
        (s.speed, s.display_name())
    };
    let (result, snapshot) = {
        let mut pet = state.pet.lock().unwrap();
        pet.tick(now_ms(), speed);
        let result = f(&mut pet);
        (result, pet.clone())
    };
    if let Err(e) = state.store.save(&snapshot) {
        eprintln!("failed to save pet: {e}");
    }
    let _ = app.emit("stats", &snapshot);
    tray::show_stats(app, &name, &snapshot);
    result
}

/// Runs an action and tells the UI how the pet reacted, whoever triggered it.
pub fn perform(app: &AppHandle, action: fn(&mut Pet) -> Outcome) -> Outcome {
    let outcome = with_pet(app, action);
    let _ = app.emit("reaction", &outcome);
    outcome
}

#[tauri::command]
fn get_pet(app: AppHandle) -> Pet {
    with_pet(&app, |pet| pet.clone())
}

#[tauri::command]
fn feed(app: AppHandle) -> Outcome {
    perform(&app, Pet::feed)
}

#[tauri::command]
fn play(app: AppHandle) -> Outcome {
    perform(&app, Pet::play)
}

#[tauri::command]
fn sleep(app: AppHandle) -> Outcome {
    perform(&app, Pet::toggle_sleep)
}

/// Asks Claude for a reply to `message`, or for an unprompted pep talk when it's None.
/// One call at a time, and rate-limited so a chatty afternoon doesn't eat the subscription.
/// What kind of answer is wanted.
enum Ask {
    /// An unprompted pep talk.
    Nudge,
    Message(String),
    /// A message about the screen: the screenshot is already saved in the brain folder.
    Screen(String),
    /// The first "bonjour" of the morning.
    Briefing(String),
}

async fn ask_pet(app: &AppHandle, state: &AppState, ask: Ask) -> Result<String, String> {
    if state.thinking.swap(true, Ordering::SeqCst) {
        return Err("hold on, i'm still thinking…".into());
    }
    let _thinking = Thinking(&state.thinking);
    if let Err(wait) = state.limiter.lock().unwrap().try_acquire(Instant::now()) {
        return Err(format!("my brain needs a break… try again in {}s", wait.as_secs().max(1)));
    }

    let (name, model) = {
        let s = state.settings.lock().unwrap();
        (s.display_name(), s.effective_model())
    };
    let status = with_pet(app, |pet| pet.describe());
    let today = state.reminders.lock().unwrap().today(chrono::Local::now());
    let (prompt, mode) = {
        let notes = state.notes.lock().unwrap();
        let history = state.history.lock().unwrap();
        match &ask {
            Ask::Nudge => (chat::build_nudge_prompt(&name, &status, &notes, &history), chat::Mode::Web),
            Ask::Message(m) => (chat::with_memory_hint(chat::build_prompt(&name, &status, &notes, &history, m)), chat::Mode::Web),
            Ask::Screen(m) => {
                let m = format!(
                    "{m}\n\n[screen] A screenshot of the human's screen right now is saved at ./{}. Read that file to see it, then answer about it.",
                    screen::FILE
                );
                (chat::build_prompt(&name, &status, &notes, &history, &m), chat::Mode::Screen)
            }
            Ask::Briefing(m) => (chat::build_briefing_prompt(&name, &status, &notes, &history, m, &today), chat::Mode::Web),
        }
    };
    let answer = chat::ask_claude(&state.brain(), &model, &prompt, mode).await?;
    let (reply, facts) = chat::take_memories(&answer.text);
    // Only facts from a turn without web pages: a page can't plant "memories".
    if matches!(ask, Ask::Message(_)) && !answer.used_tools && !facts.is_empty() {
        let mut notes = state.notes.lock().unwrap();
        if facts.iter().fold(false, |added, f| notes.add_auto(f) || added) {
            let _ = notes.save(&state.data_dir);
        }
    }

    // Pep talks aren't part of the conversation, so they don't push real turns out of memory.
    match ask {
        Ask::Nudge => {}
        Ask::Message(said) | Ask::Screen(said) | Ask::Briefing(said) => state.update_history(|h| h.push(said, reply.clone())),
    }
    Ok(reply)
}

/// The monitor under the mouse, in physical pixels: (x, y, width, height).
fn screen_under_cursor(app: &AppHandle) -> Option<(i32, i32, i32, i32)> {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())?;
    let (pos, size) = (monitor.position(), monitor.size());
    Some((pos.x, pos.y, size.width as i32, size.height as i32))
}

/// "lance discord": opens the app from the human's own shortcuts, without asking Claude.
/// None when the message isn't a launch request (or names something that isn't there and
/// reads more like a sentence), so it goes to Claude as usual.
fn launch(app: &AppHandle, state: &AppState, message: &str) -> Option<Result<String, String>> {
    use tauri_plugin_opener::OpenerExt;
    let pet_name = state.settings.lock().unwrap().display_name();
    let wanted = launcher::requested_app(message, &pet_name)?;
    let Some((name, path)) = launcher::find(&wanted, &launcher::shortcut_dirs()) else {
        if wanted.split(' ').count() > 3 {
            return None;
        }
        return Some(Err(format!("je ne trouve pas « {wanted} » dans ton menu Démarrer ni sur ton bureau")));
    };
    Some(match app.opener().open_path(path.to_string_lossy(), None::<&str>) {
        Ok(()) => {
            with_pet(app, Pet::chatted);
            Ok(format!("c'est parti, j'ouvre {name} !"))
        }
        Err(e) => Err(format!("je n'ai pas réussi à ouvrir {name} ({e})")),
    })
}

/// "briefing" any time, or the first "bonjour" between 5h and noon.
fn first_hello_of_the_morning(state: &AppState, message: &str, name: &str) -> bool {
    use chrono::Timelike;
    let now = chrono::Local::now();
    let mut briefed = state.briefed.lock().unwrap();
    let asked = crate::launcher::command_text(message, name).contains("briefing");
    let morning = (5..12).contains(&now.hour()) && *briefed != Some(now.date_naive());
    if asked || morning {
        *briefed = Some(now.date_naive());
    }
    asked || morning
}

/// Reminders and timers, understood without Claude.
fn remind(state: &AppState, message: &str) -> Option<Result<String, String>> {
    use reminders::Request;
    let name = state.settings.lock().unwrap().display_name();
    let now = chrono::Local::now();
    let request = match reminders::parse(message, &name, now)? {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    let mut list = state.reminders.lock().unwrap();
    let reply = match request {
        Request::Add(r) => list.add(r.clone()).map(|_| reminders::confirm(&r, now)),
        Request::List => Ok(list.describe(now)),
        Request::Cancel(Some(n)) => list.remove(n).map(|r| format!("rappel annulé : {}", r.text)).ok_or_else(|| format!("pas de rappel n°{n}")),
        Request::Cancel(None) => {
            list.clear();
            Ok("tous les rappels sont annulés".into())
        }
        Request::CancelTimers => Ok(match list.clear_timers() {
            0 => "aucun minuteur en cours".into(),
            n => format!("{n} minuteur(s) annulé(s)"),
        }),
    };
    if let Err(e) = list.save(&state.data_dir) {
        eprintln!("failed to save reminders: {e}");
    }
    Some(reply)
}

#[tauri::command]
async fn chat(app: AppHandle, state: State<'_, AppState>, message: String) -> Result<String, String> {
    let message = chat::clean_message(&message)?;
    // Things to do on the computer are handled here and never reach Claude.
    let name = state.settings.lock().unwrap().display_name();
    if let Some(action) = actions::parse(&message, &name) {
        let reply = tauri::async_runtime::spawn_blocking(move || actions::run(action)).await.map_err(|e| e.to_string())?;
        with_pet(&app, Pet::chatted);
        return reply;
    }
    if let Some(reply) = launch(&app, &state, &message) {
        return reply;
    }
    if let Some(reply) = remind(&state, &message) {
        return reply;
    }
    let ask = if screen::wanted(&message) {
        let (x, y, w, h) = screen_under_cursor(&app).ok_or("je ne trouve pas ton écran")?;
        let brain = state.brain();
        tauri::async_runtime::spawn_blocking(move || screen::capture(&brain, x, y, w, h)).await.map_err(|e| e.to_string())??;
        Ask::Screen(message)
    } else if chat::is_greeting(&message, &name) && first_hello_of_the_morning(&state, &message, &name) {
        Ask::Briefing(message)
    } else {
        Ask::Message(message)
    };
    let reply = ask_pet(&app, &state, ask).await?;
    with_pet(&app, Pet::chatted);
    Ok(reply)
}

/// Every `motivate` minutes, an awake pet pipes up with encouragement.
async fn pep_talk_loop(app: AppHandle) {
    let mut last = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let state = app.state::<AppState>();
        let minutes = state.settings.lock().unwrap().motivate;
        let asleep = state.pet.lock().unwrap().asleep;
        if minutes == 0 || asleep || last.elapsed() < Duration::from_secs(minutes as u64 * 60) {
            continue;
        }
        last = Instant::now();
        let text = ask_pet(&app, &state, Ask::Nudge)
            .await
            .unwrap_or_else(|_| chat::fallback_pep(now_ms()).to_string());
        let _ = app.emit("nudge", text);
    }
}

/// Saves and exits. The pet window calls this once its goodbye animation has played.
#[tauri::command]
fn quit(app: AppHandle) {
    with_pet(&app, |_| ());
    app.exit(0);
}

/// Asks the pet to say goodbye before quitting, or quits right away if it's hidden.
/// Exits anyway after a few seconds in case the window never answers.
pub fn farewell(app: &AppHandle) {
    let visible = app
        .get_webview_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false));
    if !visible || app.emit_to("main", "farewell", ()).is_err() {
        quit(app.clone());
        return;
    }
    let app = app.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(6));
        quit(app);
    });
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

/// Handles chat-box input starting with `/`. These never reach Claude.
#[tauri::command]
async fn command(app: AppHandle, state: State<'_, AppState>, input: String) -> Result<String, String> {
    match settings::parse(&input)? {
        Command::Help => Ok(settings::HELP.into()),
        Command::ShowSettings => Ok(state.settings.lock().unwrap().describe()),
        Command::Set(key, value) => {
            let updated = {
                let mut s = state.settings.lock().unwrap();
                let msg = s.set(&key, &value)?;
                if let Err(e) = s.save(&state.data_dir) {
                    eprintln!("failed to save settings: {e}");
                }
                (msg, s.clone())
            };
            if key == "skin" {
                // A different body may come with a different personality; start fresh.
                state.update_history(History::clear);
            }
            apply_settings(&app, &updated.1);
            Ok(updated.0)
        }
        Command::Personality { reset: false } => {
            personality::open_editor(&app);
            Ok("opening my personality editor… save it and i'll change on the next chat".into())
        }
        Command::Personality { reset: true } => {
            chat::reset_personality(&state.brain(), &state.skin()).map_err(|e| format!("couldn't reset: {e}"))?;
            Ok("personality restored to the default. hi again!".into())
        }
        Command::Motivate => {
            let text = match ask_pet(&app, &state, Ask::Nudge).await {
                Ok(text) => text,
                Err(_) => chat::fallback_pep(now_ms()).to_string(),
            };
            Ok(text)
        }
        Command::Doctor => chat::check_cli().await,
        Command::Stats => Ok(with_pet(&app, |pet| pet.describe())),
        Command::Clear => {
            state.update_history(History::clear);
            Ok("conversation forgotten. who are you again? :)".into())
        }
        Command::Remember(note) => {
            let mut notes = state.notes.lock().unwrap();
            notes.add(&note)?;
            notes.save(&state.data_dir).map_err(|e| format!("couldn't save that: {e}"))?;
            Ok(format!("noted! i'll remember that ({}/{})", notes.list().len(), chat::MAX_NOTES))
        }
        Command::Memories => {
            let notes = state.notes.lock().unwrap();
            if notes.list().is_empty() && notes.auto().is_empty() {
                return Ok("je ne retiens encore rien. apprends-moi avec /remember <fait>".into());
            }
            let asked = notes.list().iter().map(|n| n.to_string());
            let noticed = notes.auto().iter().map(|n| format!("{n} (retenu tout seul)"));
            Ok(asked.chain(noticed).enumerate().map(|(i, n)| format!("{}. {n}", i + 1)).collect::<Vec<_>>().join("\n"))
        }
        Command::Forget => {
            let mut notes = state.notes.lock().unwrap();
            notes.clear();
            notes.save(&state.data_dir).map_err(|e| format!("couldn't forget: {e}"))?;
            Ok("all my notes are gone. fresh brain!".into())
        }
        Command::Reminders => Ok(state.reminders.lock().unwrap().describe(chrono::Local::now())),
        Command::Cancel(which) => {
            let mut list = state.reminders.lock().unwrap();
            let reply = match which {
                Some(n) => list.remove(n).map(|r| format!("rappel annulé : {}", r.text)).ok_or_else(|| format!("pas de rappel n°{n}")),
                None => {
                    list.clear();
                    Ok("tous les rappels sont annulés".into())
                }
            };
            let _ = list.save(&state.data_dir);
            reply
        }
        Command::Reset { confirmed: false } => {
            Err("this hatches a brand-new pet with fresh stats. type /reset confirm to do it".into())
        }
        Command::Reset { confirmed: true } => {
            with_pet(&app, |pet| *pet = Pet::new(now_ms()));
            state.update_history(History::clear);
            Ok("*crack* …a fresh new pet! hello!".into())
        }
    }
}

/// The pet's voice: an MP3 from ElevenLabs (or its cache). Errors make the window use the
/// Windows voice instead; "not_configured" means there's no ElevenLabs key.
#[tauri::command]
async fn tts(app: AppHandle, state: State<'_, AppState>, text: String) -> Result<tauri::ipc::Response, String> {
    let (key, voice, model) = {
        let s = state.settings.lock().unwrap();
        (s.eleven_key.clone(), s.eleven_voice.clone(), tts::model_id(&s.eleven_model).to_string())
    };
    let cache = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("voice");
    let text: String = text.chars().take(1500).collect();
    tauri::async_runtime::spawn_blocking(move || tts::speak(&cache, &key, &voice, &model, &text))
        .await
        .map_err(|e| e.to_string())?
        .map(tauri::ipc::Response::new)
}

/// Ctrl+Space: brings the pet up with the chat box ready, then the window starts dictation.
fn summon(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
    if voice_in::model_path(app).is_some() {
        // Listen with the local speech model: talk, and it sends by itself.
        app.state::<voice_in::Ears>().arm();
        let _ = app.emit("listen", voice_in::Listen { state: "listening", text: String::new() });
    } else {
        let _ = app.emit("dictate", ());
    }
}

/// The 🎙 button: same as Ctrl+Space.
#[tauri::command]
fn listen_now(app: AppHandle) {
    summon(&app);
}

#[tauri::command]
fn cancel_listen(ears: State<'_, voice_in::Ears>) {
    ears.cancel();
}

/// The window says when the pet starts and stops talking, so the microphone ignores it.
#[tauri::command]
fn set_speaking(ears: State<'_, voice_in::Ears>, on: bool) {
    ears.set_speaking(on);
}

#[tauri::command]
fn start_dictation() -> Result<(), String> {
    actions::start_dictation()
}

/// Every few seconds: anything due is said out loud (the window pops up if it was hidden).
fn reminder_loop(app: AppHandle) {
    loop {
        thread::sleep(Duration::from_secs(5));
        let state = app.state::<AppState>();
        let now = chrono::Local::now().timestamp();
        let due = {
            let mut list = state.reminders.lock().unwrap();
            let due = list.take_due(now);
            if !due.is_empty() {
                let _ = list.save(&state.data_dir);
            }
            due
        };
        for r in due {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
            }
            let _ = app.emit("reminder", reminders::announce(&r, now));
        }
    }
}

fn apply_autostart(app: &AppHandle, on: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let launcher = app.autolaunch();
    let result = if on { launcher.enable() } else { launcher.disable() };
    if let Err(e) = result {
        eprintln!("couldn't change autostart: {e}");
    }
}

/// Pushes settings that affect the window, the limiter and the UI.
fn apply_settings(app: &AppHandle, s: &Settings) {
    let state = app.state::<AppState>();
    state.limiter.lock().unwrap().set_max_calls(s.hourly_limit);
    if cfg!(not(debug_assertions)) {
        apply_autostart(app, s.autostart);
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_always_on_top(s.on_top);
    }
    let ears = app.state::<voice_in::Ears>();
    ears.set_wake(s.listen);
    ears.set_name(s.display_name());
    let _ = app.emit("settings", s);
    with_pet(app, |_| ()); // refresh the tray tooltip with the new name
}

/// Parks the pet in the bottom-right corner of the monitor it opens on.
fn place_bottom_right(window: &WebviewWindow) -> tauri::Result<()> {
    if let Some(monitor) = window.current_monitor()? {
        let area = monitor.work_area();
        let size = window.outer_size()?;
        let margin = 24;
        window.set_position(PhysicalPosition::new(
            area.position.x + area.size.width as i32 - size.width as i32 - margin,
            area.position.y + area.size.height as i32 - size.height as i32,
        ))?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            get_pet,
            feed,
            play,
            sleep,
            chat,
            get_settings,
            command,
            quit,
            personality::open_personality,
            personality::personality_info,
            personality::get_personality,
            personality::save_personality,
            personality::reset_personality,
            personality::open_personality_file,
            tts,
            start_dictation,
            listen_now,
            cancel_listen,
            set_speaking,
        ])
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let settings = Settings::load(&data_dir);
            let store = Store::new(&data_dir);
            let pet = store.load(now_ms(), settings.speed);
            let hourly_limit = settings.hourly_limit;
            app.manage(AppState {
                pet: Mutex::new(pet),
                store,
                settings: Mutex::new(settings.clone()),
                data_dir: data_dir.clone(),
                limiter: Mutex::new(RateLimiter::new(
                    Duration::from_secs(4),
                    hourly_limit,
                    Duration::from_secs(60 * 60),
                )),
                history: Mutex::new(History::load(&data_dir)),
                notes: Mutex::new(Notes::load(&data_dir)),
                reminders: Mutex::new({
                    let mut list = reminders::Reminders::load(&data_dir);
                    list.catch_up(chrono::Local::now().timestamp());
                    list
                }),
                thinking: AtomicBool::new(false),
                briefed: Mutex::new(None),
            });
            app.manage(voice_in::Ears::new(settings.listen, settings.display_name()));
            voice_in::start(app.handle().clone());

            let window = app
                .get_webview_window("main")
                .expect("main window is declared in tauri.conf.json");
            place_bottom_right(&window)?;
            window.set_always_on_top(settings.on_top)?;
            window.show()?;
            tray::build(app.handle())?;
            if cfg!(not(debug_assertions)) {
                apply_autostart(app.handle(), settings.autostart);
            }
            {
                use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
                let registered = app.global_shortcut().on_shortcut("ctrl+space", |app, _, event| {
                    if event.state == ShortcutState::Pressed {
                        summon(app);
                    }
                });
                if let Err(e) = registered {
                    eprintln!("couldn't register Ctrl+Space: {e}");
                }
            }
            let reminders_app = app.handle().clone();
            thread::spawn(move || reminder_loop(reminders_app));

            tauri::async_runtime::spawn(pep_talk_loop(app.handle().clone()));
            let handle = app.handle().clone();
            thread::spawn(move || loop {
                with_pet(&handle, |_| ());
                thread::sleep(TICK);
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Claude Pet");
}
