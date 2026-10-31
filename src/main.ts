import {
  act,
  chat,
  getPet,
  getSettings,
  cancelListen,
  listenNow,
  onDictate,
  onHeard,
  onListen,
  onNudge,
  onReaction,
  onReminder,
  onSettings,
  onStats,
  openPersonality,
  quit,
  runCommand,
  setSpeaking,
  startDictation,
  ttsAudio,
  type Pet,
  type Reaction,
  type Settings,
} from "./api";
import { listen } from "@tauri-apps/api/event";
import { Menu } from "@tauri-apps/api/menu";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Panel } from "./panel";
import { PetView, type Mood } from "./pet";

const bubbleEl = document.getElementById("bubble")!;
const bubbleText = bubbleEl.querySelector<HTMLElement>(".bubble-text")!;
let bubbleTimer: number | undefined;

/**
 * Shows a speech bubble for long enough to read it (about 15 characters a second).
 * It stays up while hovered or being read aloud, so long replies can be scrolled.
 * `ms = 0` keeps it up.
 */
function say(text: string, ms = Math.min(60000, 3000 + text.length * 65)) {
  bubbleEl.classList.remove("thinking");
  bubbleText.textContent = text;
  bubbleText.scrollTop = 0;
  bubbleEl.classList.add("show");
  clearTimeout(bubbleTimer);
  if (ms > 0) bubbleTimer = window.setTimeout(hideBubble, ms);
}

function hideBubble() {
  if (bubbleEl.matches(":hover") || speaking()) {
    bubbleTimer = window.setTimeout(hideBubble, 1500);
    return;
  }
  bubbleEl.classList.remove("show");
}

// ---------- voice: ElevenLabs when it's set up, otherwise the Windows voice ----------

const tts = "speechSynthesis" in window ? window.speechSynthesis : undefined;
let voiceOn = true;
let elevenOn = false;
/** "elevenlabs", "windows" or "mix" (ElevenLabs only for short answers). */
let engine = "elevenlabs";
let cartoon = false;
let cartoonPitch = 1.7;
/** Answers up to this long use ElevenLabs in "mix" mode; longer ones the free Windows voice. */
const MIX_MAX = 180;
let frenchVoice: SpeechSynthesisVoice | undefined;
/** Long answers: only the start is read aloud (the bubble has the rest), to save credits. */
const VOICE_MAX = 600;

/** Prefers a French voice, and the natural-sounding ones when Windows has them. */
function pickVoice() {
  const voices = tts?.getVoices() ?? [];
  const fr = voices.filter((v) => v.lang.toLowerCase().startsWith("fr"));
  frenchVoice =
    fr.find((v) => /natural|neural|online/i.test(v.name) && /fr-fr/i.test(v.lang)) ??
    fr.find((v) => /fr-fr/i.test(v.lang)) ??
    fr[0];
}
if (tts) {
  pickVoice();
  tts.addEventListener("voiceschanged", pickVoice);
}

let audio: AudioContext | undefined;
let clip: AudioBufferSourceNode | undefined;
/** Bumped whenever speech starts or stops, so a late ElevenLabs answer is dropped. */
let voiceTurn = 0;
let lastVoiceProblem = "";
/** The line being read by the Windows voice, so a cancelled one can't touch the bubble later. */
let current: SpeechSynthesisUtterance | undefined;

const speaking = () => (tts?.speaking ?? false) || clip !== undefined;

/** Tells the listener the pet's own voice is playing, so it doesn't hear itself. */
let micMuted = false;
function muteMic(on: boolean) {
  if (on === micMuted) return;
  micMuted = on;
  void setSpeaking(on).catch(() => {});
}

function stopVoice() {
  voiceTurn++;
  current = undefined;
  tts?.cancel();
  const playing = clip;
  clip = undefined;
  playing?.stop();
  pet.settle();
  muteMic(false);
}

/** What's worth saying out loud: no links, *sound effects*, emoji or markdown symbols. */
function forSpeech(text: string) {
  return text
    .replace(/https?:\/\/\S+/g, "")
    .replace(/\*[^*\n]{1,30}\*/g, "")
    .replace(/[\p{Extended_Pictographic}\u{FE0F}\u{200D}]/gu, "")
    .replace(/[#*_`>|~]/g, "")
    .replace(/^\s*-\s+/gm, "")
    .replace(/\s+/g, " ")
    .trim();
}

/** The first sentences of `text` that fit in `max` characters. */
function firstPart(text: string, max: number) {
  if (text.length <= max) return text;
  const cut = text.slice(0, max);
  const end = Math.max(cut.lastIndexOf(". "), cut.lastIndexOf("! "), cut.lastIndexOf("? "));
  return end > max / 3 ? cut.slice(0, end + 1) : `${cut.slice(0, cut.lastIndexOf(" "))}…`;
}

/** Talking is over: stop moving the mouth, and let the bubble go a few seconds later. */
function doneTalking() {
  pet.settle();
  // A short pause first: the end of the voice is still in the room for a moment.
  setTimeout(() => {
    if (!speaking()) muteMic(false);
  }, 400);
  clearTimeout(bubbleTimer);
  bubbleTimer = window.setTimeout(hideBubble, 4000);
}

function windowsVoice(line: string, turn: number) {
  if (!tts || turn !== voiceTurn) return pet.settle();
  const utterance = new SpeechSynthesisUtterance(line);
  utterance.lang = frenchVoice?.lang ?? "fr-FR";
  if (frenchVoice) utterance.voice = frenchVoice;
  // Cartoon: higher and a little quicker, like an animated sidekick.
  utterance.rate = cartoon ? 1.12 : 1.05;
  utterance.pitch = cartoon ? cartoonPitch : 1;
  utterance.onend = () => {
    if (utterance !== current) return;
    current = undefined;
    doneTalking();
  };
  current = utterance;
  muteMic(true);
  tts.speak(utterance);
}

/** Splits off the first sentence, so the voice can start while the rest is being made. */
function splitFirst(line: string): string[] {
  const m = line.match(/^(.{15,220}?[.!?…])\s+(\S[\s\S]*)$/);
  return m ? [m[1], m[2]] : [line];
}

/** What's left to say if ElevenLabs fails halfway (the Windows voice finishes it). */
let unspoken = "";

function playClip(buffer: AudioBuffer, last: boolean): Promise<void> {
  return new Promise((resolve) => {
    const source = audio!.createBufferSource();
    source.buffer = buffer;
    // ElevenLabs can't change its pitch, so cartoon mode plays it a bit faster (and higher).
    if (cartoon) source.playbackRate.value = 1 + (cartoonPitch - 1) * 0.25;
    source.connect(audio!.destination);
    source.onended = () => {
      if (clip === source) {
        clip = undefined;
        if (last) doneTalking();
      }
      resolve();
    };
    clip = source;
    muteMic(true);
    source.start();
  });
}

async function elevenVoice(line: string, turn: number) {
  const parts = splitFirst(line);
  unspoken = line;
  // Both parts are asked for at once; the first plays as soon as it's ready.
  const pending = parts.map((p) => ttsAudio(p));
  pending.forEach((p) => p.catch(() => {}));
  audio ??= new AudioContext();
  if (audio.state === "suspended") await audio.resume();
  for (const [i, clipBytes] of pending.entries()) {
    const buffer = await audio.decodeAudioData(await clipBytes);
    if (turn !== voiceTurn) return;
    await playClip(buffer, i === parts.length - 1);
    if (turn !== voiceTurn) return;
    unspoken = parts.slice(i + 1).join(" ");
  }
}

function readAloud(text: string) {
  stopVoice();
  const line = firstPart(forSpeech(text), VOICE_MAX);
  if (!line || !voiceOn) return;
  const turn = voiceTurn;
  // The mouth moves for as long as the voice is going (doneTalking stops it).
  pet.play("talk", 120000);
  const useEleven = elevenOn && (engine === "elevenlabs" || (engine === "mix" && line.length <= MIX_MAX));
  if (!useEleven) return windowsVoice(line, turn);
  elevenVoice(line, turn)
    .then(() => {
      lastVoiceProblem = "";
    })
    .catch((err) => {
      if (turn !== voiceTurn) return;
      const why = String(err);
      // Each new problem is shown once, so a fixed one doesn't keep nagging.
      if (why !== lastVoiceProblem && !why.includes("not_configured")) {
        lastVoiceProblem = why;
        bubbleText.textContent += `\n\n🔈 ${why} — voix Windows en attendant`;
      }
      windowsVoice(unspoken || line, turn);
    });
}

/** The pet "speaks" a line, out loud too when `aloud` and the voice is on. */
function speak(text: string, aloud = false) {
  say(text);
  if (aloud && voiceOn) readAloud(text);
  else pet.play("talk", Math.min(3000, 400 + text.length * 25));
}

function applySettings(s: Settings) {
  pet.setSkin(s.skin);
  document.body.dataset.theme = s.theme;
  voiceOn = s.voice;
  elevenOn = s.eleven_key !== "";
  engine = s.voice_engine;
  cartoon = s.voice_style === "cartoon";
  cartoonPitch = s.voice_pitch;
  if (!voiceOn) stopVoice();
}

function moodOf(p: Pet): Mood {
  if (p.asleep) return "sleeping";
  if (p.hunger < 25) return "hungry";
  if (p.energy < 20) return "tired";
  if (p.mood < 30) return "sad";
  if (p.mood > 70) return "happy";
  return "neutral";
}

// Clicking the pet makes it wiggle, hushes it if it was talking, and stops listening.
const pet = new PetView(document.getElementById("pet")!, () => {
  stopVoice();
  if (document.body.classList.contains("listening")) {
    document.body.classList.remove("listening");
    void cancelListen();
    hideBubble();
  }
  pet.play("wiggle");
});
const panel = new Panel(
  document.getElementById("panel")!,
  (action) => void act(action),
  (msg) => void talk(msg),
  () => void listenNow(),
);

async function talk(msg: string, spoken = false) {
  if (/^\/(bye|quit|exit)\s*$/i.test(msg)) return farewell();
  if (msg.startsWith("/")) return command(msg);
  panel.setThinking(true);
  stopVoice();
  // When it was said out loud, show what was understood while the pet thinks.
  bubbleText.textContent = spoken ? `« ${msg} »` : "";
  bubbleEl.classList.add("show", "thinking");
  clearTimeout(bubbleTimer);
  try {
    // Spoken to → answers out loud; typed to → answers in the bubble only.
    speak(await chat(msg), spoken);
  } catch (err) {
    pet.play("wiggle");
    say(String(err));
  } finally {
    panel.setThinking(false);
  }
}

/** Slash commands are handled by the app itself, never sent to Claude. */
async function command(input: string) {
  const pep = /^\/(motivate|pep)\b/i.test(input);
  try {
    const out = await runCommand(input);
    if (pep) speak(out);
    else say(out, 10000);
  } catch (err) {
    say(String(err), 6000);
  }
}

let leaving = false;

/** Goodbye: the pet waves, flops, its little soul floats away, then the app closes. */
async function farewell() {
  if (leaving) return;
  leaving = true;
  stopVoice();
  document.body.classList.add("leaving");
  say("bye bye! see you soon", 0);
  await pet.playFinal("bye");
  await quit();
}

let latest: Pet | undefined;

/** Right-click on the pet: the everyday actions, plus a way out. */
async function showPetMenu() {
  if (leaving) return;
  const menu = await Menu.new({
    items: [
      { id: "feed", text: "Feed", action: () => void act("feed") },
      { id: "play", text: "Play", action: () => void act("play") },
      { id: "sleep", text: latest?.asleep ? "Wake up" : "Sleep", action: () => void act("sleep") },
      { item: "Separator" },
      { id: "personality", text: "Edit personality…", action: () => void openPersonality() },
      { id: "hide", text: "Hide (tray icon brings me back)", action: () => void getCurrentWindow().hide() },
      { item: "Separator" },
      { id: "bye", text: "Bye bye 👋", action: () => void farewell() },
    ],
  });
  await menu.popup();
}

document.addEventListener("contextmenu", (e) => {
  // Keep the browser's menu for the text fields; everything else gets ours.
  if ((e.target as HTMLElement).closest("input, textarea")) return;
  e.preventDefault();
  if ((e.target as HTMLElement).closest("#pet")) void showPetMenu();
});

function render(p: Pet) {
  latest = p;
  pet.setMood(moodOf(p));
  panel.update(p);
}

const ANIM: Partial<Record<Reaction["action"], "eat" | "play">> = { feed: "eat", play: "play" };

function react(r: Reaction) {
  const anim = r.ok ? ANIM[r.action] : undefined;
  if (anim) pet.play(anim);
  else if (!r.ok) pet.play("wiggle");
  say(r.message);
}

void onStats(render);
void onReaction(react);
void onSettings(applySettings);
void onNudge(speak);
// Reminders pop up, wiggle and are said out loud.
void onReminder((text) => {
  pet.play("wiggle");
  setTimeout(() => speak(text, true), 600);
});
/** A short rising "ding": the pet is listening. */
function ding() {
  try {
    audio ??= new AudioContext();
    const t = audio.currentTime;
    const osc = audio.createOscillator();
    const gain = audio.createGain();
    osc.frequency.setValueAtTime(660, t);
    osc.frequency.exponentialRampToValueAtTime(990, t + 0.12);
    gain.gain.setValueAtTime(0.0001, t);
    gain.gain.exponentialRampToValueAtTime(0.18, t + 0.02);
    gain.gain.exponentialRampToValueAtTime(0.0001, t + 0.22);
    osc.connect(gain).connect(audio.destination);
    osc.start(t);
    osc.stop(t + 0.25);
  } catch {
    // no sound, no problem
  }
}

void onListen(({ state, text }) => {
  document.body.classList.toggle("listening", state === "listening");
  if (state === "listening") {
    stopVoice();
    muteMic(false);
    ding();
    pet.play("wiggle");
    say("🎙️ je t'écoute…", 0);
  } else if (state === "thinking") {
    bubbleText.textContent = "🎙️";
    bubbleEl.classList.add("show", "thinking");
    clearTimeout(bubbleTimer);
  } else if (state === "error") {
    say(`🎙️ ${text}`, 6000);
  } else if (text) {
    say(text, 2500);
  } else if (bubbleText.textContent?.startsWith("🎙️")) {
    hideBubble();
  }
});
void onHeard((text) => {
  document.body.classList.remove("listening");
  // A little "ding": it heard its name and understood.
  ding();
  void talk(text, true);
});

// Without the speech model, F9 falls back to Windows voice typing in the chat box.
void onDictate(() => {
  stopVoice();
  panel.focusInput();
  setTimeout(() => startDictation().catch((err) => say(String(err), 5000)), 250);
});
void listen("farewell", () => void farewell()); // Quit from the tray
void Promise.all([getPet(), getSettings()]).then(([p, s]) => {
  applySettings(s);
  render(p);
  // First-run help for colleagues who don't have Claude Code yet.
  runCommand("/doctor").catch((err) => say(String(err), 0));
  if (p.asleep) say("zzz…");
  else if (s.skin === "spine") say("oh good, it's you. spiny's here, try to contain yourself. /help if you must");
  else say(`coucou, c'est ${s.name} ! pose-moi une question, ou tape /help pour les commandes`);
});
