import {
  act,
  chat,
  getPet,
  getSettings,
  onNudge,
  onReaction,
  onSettings,
  onStats,
  openPersonality,
  quit,
  runCommand,
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

// ---------- voice: replies read aloud with the system's text-to-speech (Windows voices) ----------

const tts = "speechSynthesis" in window ? window.speechSynthesis : undefined;
let voiceOn = true;
let frenchVoice: SpeechSynthesisVoice | undefined;

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

const speaking = () => tts?.speaking ?? false;
/** The line being read right now, so a cancelled one can't touch the bubble later. */
let current: SpeechSynthesisUtterance | undefined;

function stopVoice() {
  current = undefined;
  tts?.cancel();
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

function readAloud(text: string) {
  if (!tts || !voiceOn) return;
  stopVoice();
  const line = forSpeech(text);
  if (!line) return;
  const utterance = new SpeechSynthesisUtterance(line);
  utterance.lang = frenchVoice?.lang ?? "fr-FR";
  if (frenchVoice) utterance.voice = frenchVoice;
  utterance.rate = 1.05;
  // Once it's done talking, the bubble can go away a few seconds later.
  utterance.onend = () => {
    if (utterance !== current) return;
    current = undefined;
    clearTimeout(bubbleTimer);
    bubbleTimer = window.setTimeout(hideBubble, 4000);
  };
  current = utterance;
  tts.speak(utterance);
}

/** The pet "speaks" a line Claude wrote, out loud too when `aloud` and the voice is on. */
function speak(text: string, aloud = false) {
  const voiced = aloud && voiceOn && !!tts;
  pet.play("talk", voiced ? Math.min(20000, 400 + text.length * 55) : Math.min(3000, 400 + text.length * 25));
  say(text);
  if (voiced) readAloud(text);
}

function applySettings(s: Settings) {
  pet.setSkin(s.skin);
  voiceOn = s.voice;
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

// Clicking the pet makes it wiggle, and hushes it if it was talking.
const pet = new PetView(document.getElementById("pet")!, () => {
  stopVoice();
  pet.play("wiggle");
});
const panel = new Panel(
  document.getElementById("panel")!,
  (action) => void act(action),
  (msg) => void talk(msg),
);

async function talk(msg: string) {
  if (/^\/(bye|quit|exit)\s*$/i.test(msg)) return farewell();
  if (msg.startsWith("/")) return command(msg);
  panel.setThinking(true);
  stopVoice();
  bubbleText.textContent = "";
  bubbleEl.classList.add("show", "thinking");
  clearTimeout(bubbleTimer);
  try {
    speak(await chat(msg), true);
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
    if (pep) speak(out, true);
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
