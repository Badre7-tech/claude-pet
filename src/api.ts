import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** Mirrors `stats::Pet` in Rust. All stats are 0–100, higher is better. */
export type Pet = {
  hunger: number;
  mood: number;
  energy: number;
  asleep: boolean;
  updated_at: number;
};

/** Mirrors `stats::Outcome`: the result of an action, from the UI or the tray. */
export type Reaction = {
  action: "feed" | "play" | "sleep" | "wake";
  ok: boolean;
  message: string;
};

/** Mirrors `settings::Settings`. Changed by typing `/settings <key> <value>`. */
export type Settings = {
  name: string;
  model: string;
  speed: number;
  on_top: boolean;
  hourly_limit: number;
  skin: string;
  motivate: number;
  voice: boolean;
  theme: string;
  autostart: boolean;
  eleven_key: string;
  eleven_voice: string;
  eleven_model: string;
  voice_engine: string;
  voice_style: string;
  voice_pitch: number;
};

export const getPet = () => invoke<Pet>("get_pet");
export const act = (action: "feed" | "play" | "sleep") => invoke<Reaction>(action);
export const getSettings = () => invoke<Settings>("get_settings");
export const runCommand = (input: string) => invoke<string>("command", { input });
export const quit = () => invoke("quit");
export const openPersonality = () => invoke("open_personality");
export const chat = (message: string) => invoke<string>("chat", { message });

export const onStats = (cb: (pet: Pet) => void) => listen<Pet>("stats", (e) => cb(e.payload));
export const onReaction = (cb: (r: Reaction) => void) =>
  listen<Reaction>("reaction", (e) => cb(e.payload));
export const onSettings = (cb: (s: Settings) => void) =>
  listen<Settings>("settings", (e) => cb(e.payload));
export const onNudge = (cb: (text: string) => void) => listen<string>("nudge", (e) => cb(e.payload));
export const onReminder = (cb: (text: string) => void) => listen<string>("reminder", (e) => cb(e.payload));
export const onDictate = (cb: () => void) => listen("dictate", () => cb());
/** An MP3 of the pet's ElevenLabs voice saying `text`. */
export const ttsAudio = (text: string) => invoke<ArrayBuffer>("tts", { text });
export const startDictation = () => invoke("start_dictation");
