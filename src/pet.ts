import { getCurrentWindow } from "@tauri-apps/api/window";
import sprites from "./sprites.json";

/** Visual states the pet can be in. Each mood loops one sprite row. */
export type Mood = "happy" | "neutral" | "sad" | "hungry" | "tired" | "sleeping";
export type Anim = keyof typeof sprites.anims;
export type Skin = keyof typeof sprites.skins;

export const SKINS = Object.keys(sprites.skins) as Skin[];

const MOOD_ANIM: Record<Mood, Anim> = {
  happy: "happy",
  neutral: "idle",
  sad: "sad",
  hungry: "hungry",
  tired: "tired",
  sleeping: "sleep",
};

const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

/** How long one pass of an animation takes, in ms. */
const length = (anim: Anim) => sprites.anims[anim].durations.reduce((sum, d) => sum + d, 0);

/** Plays the sprite sheet for the current skin: a looping mood row plus one-shot reactions. */
export class PetView {
  private sprite: HTMLElement;
  private mood: Mood = "neutral";
  private current: Anim = "idle";
  private frame = 0;
  private timer?: number;
  /** When a one-shot ends (ms timestamp), or undefined when looping the mood. */
  private oneShotUntil?: number;

  constructor(private el: HTMLElement, onClick: () => void) {
    this.sprite = document.createElement("div");
    this.sprite.className = "sprite";
    const { frame, cols, rows } = sprites;
    this.sprite.style.backgroundSize = `${cols * frame}px ${rows * frame}px`;
    el.replaceChildren(this.sprite);
    this.setSkin("blob");
    this.start("idle");
    this.enableDrag(onClick);
  }

  setSkin(skin: string) {
    const id = (SKINS as string[]).includes(skin) ? skin : "blob";
    this.sprite.style.backgroundImage = `url(/sprites/${id}.png)`;
    this.el.dataset.skin = id;
  }

  setMood(mood: Mood) {
    if (mood === this.mood) return;
    this.mood = mood;
    if (this.oneShotUntil === undefined && !this.final) this.start(MOOD_ANIM[mood]);
  }

  /**
   * Plays a reaction, then returns to the mood loop. Non-looping rows play once;
   * looping rows (like talk) repeat for `ms`.
   */
  play(anim: Anim, ms?: number) {
    if (this.final) return;
    this.oneShotUntil = performance.now() + (ms ?? (sprites.anims[anim].loop ? length(anim) * 2 : length(anim)));
    this.start(anim);
  }

  /** Ends a reaction now (e.g. the voice stopped talking) and goes back to the mood loop. */
  settle() {
    if (this.final || this.oneShotUntil === undefined) return;
    this.oneShotUntil = undefined;
    this.start(MOOD_ANIM[this.mood]);
  }

  private final = false;

  /** Plays `anim` once and stays on its last frame for good; resolves when it has finished. */
  playFinal(anim: Anim): Promise<void> {
    this.final = true;
    this.oneShotUntil = Infinity;
    this.start(anim);
    return new Promise((done) => setTimeout(done, length(anim)));
  }

  private start(anim: Anim) {
    this.current = anim;
    this.frame = 0;
    clearTimeout(this.timer);
    this.draw();
  }

  /** Shows the current frame and schedules the next one after that frame's duration. */
  private schedule() {
    clearTimeout(this.timer);
    if (reducedMotion.matches) return;
    const ms = sprites.anims[this.current].durations[this.frame];
    this.timer = window.setTimeout(() => this.step(), ms);
  }

  private step() {
    const a = sprites.anims[this.current];
    if (this.oneShotUntil !== undefined && performance.now() >= this.oneShotUntil) {
      this.oneShotUntil = undefined;
      this.start(MOOD_ANIM[this.mood]);
      return;
    }
    this.frame = a.loop ? (this.frame + 1) % a.frames : Math.min(this.frame + 1, a.frames - 1);
    this.draw();
  }

  private draw() {
    const { frame } = sprites;
    const row = sprites.anims[this.current].row;
    this.sprite.style.backgroundPosition = `${-this.frame * frame}px ${-row * frame}px`;
    this.schedule();
  }

  /** A press that moves more than a few pixels drags the window; otherwise it's a click. */
  private enableDrag(onClick: () => void) {
    let start: { x: number; y: number } | null = null;

    this.el.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      start = { x: e.screenX, y: e.screenY };
    });
    this.el.addEventListener("pointermove", (e) => {
      if (!start) return;
      if (Math.hypot(e.screenX - start.x, e.screenY - start.y) > 4) {
        start = null;
        void getCurrentWindow().startDragging();
      }
    });
    this.el.addEventListener("pointerup", () => {
      if (start) onClick();
      start = null;
    });
  }
}
