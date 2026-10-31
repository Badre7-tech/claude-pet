import type { Pet } from "./api";
import { suggest, type Suggestion } from "./complete";

type Stat = "hunger" | "mood" | "energy";
const STATS: { key: Stat; label: string }[] = [
  { key: "hunger", label: "food" },
  { key: "mood", label: "mood" },
  { key: "energy", label: "energy" },
];

export type PanelAction = "feed" | "play" | "sleep";

/** Stat bars, action buttons and the chat box shown while hovering the pet. */
export class Panel {
  private bars = new Map<Stat, { fill: HTMLElement; value: HTMLElement }>();
  private sleepBtn: HTMLButtonElement;
  private input: HTMLInputElement;
  private sendBtn: HTMLButtonElement;

  constructor(
    el: HTMLElement,
    onAction: (a: PanelAction) => void,
    onChat: (msg: string) => void,
    onMic: () => void,
  ) {
    el.innerHTML = /* html */ `
      <div class="stats">
        ${STATS.map(
          (s) => `
          <div class="stat" data-stat="${s.key}">
            <span class="stat-label">${s.label}</span>
            <span class="stat-track"><span class="stat-fill"></span></span>
            <span class="stat-value">0</span>
          </div>`,
        ).join("")}
      </div>
      <div class="actions">
        <button type="button" data-action="feed">feed</button>
        <button type="button" data-action="play">play</button>
        <button type="button" data-action="sleep">sleep</button>
      </div>
      <ul class="suggest" role="listbox" aria-label="Command suggestions" hidden></ul>
      <form class="chat" autocomplete="off">
        <input name="msg" maxlength="2000" placeholder="pose-moi une question… ou /help" aria-label="Message to your pet" />
        <button type="button" class="mic" aria-label="Parler" title="Parler (F9)">🎙</button>
        <button type="submit" aria-label="Send">↑</button>
      </form>`;

    for (const s of STATS) {
      const row = el.querySelector<HTMLElement>(`[data-stat="${s.key}"]`)!;
      this.bars.set(s.key, {
        fill: row.querySelector(".stat-fill")!,
        value: row.querySelector(".stat-value")!,
      });
    }

    el.querySelectorAll<HTMLButtonElement>("[data-action]").forEach((btn) =>
      btn.addEventListener("click", () => onAction(btn.dataset.action as PanelAction)),
    );
    // Buttons never take the keyboard focus: otherwise a later Space or Enter, meant for
    // another app, would press them again.
    el.querySelectorAll<HTMLButtonElement>("button").forEach((btn) => {
      btn.tabIndex = -1;
      btn.addEventListener("mousedown", (e) => e.preventDefault());
    });
    this.sleepBtn = el.querySelector('[data-action="sleep"]')!;

    const form = el.querySelector<HTMLFormElement>(".chat")!;
    this.input = form.querySelector("input")!;
    this.sendBtn = form.querySelector('button[type="submit"]')!;
    form.querySelector(".mic")!.addEventListener("click", () => onMic());
    this.list = el.querySelector(".suggest")!;
    form.addEventListener("submit", (e) => {
      e.preventDefault();
      const msg = this.input.value.trim();
      // Slash commands are local, so they still work while the pet is thinking.
      if (!msg || (this.sendBtn.disabled && !msg.startsWith("/"))) return;
      this.input.value = "";
      this.refreshSuggestions();
      onChat(msg);
    });
    this.input.addEventListener("input", () => this.refreshSuggestions());
    this.input.addEventListener("keydown", (e) => this.onKey(e));
    this.input.addEventListener("blur", () => setTimeout(() => this.showSuggestions([]), 120));
    this.list.addEventListener("pointerdown", (e) => {
      const li = (e.target as HTMLElement).closest<HTMLElement>("[data-i]");
      if (!li) return;
      e.preventDefault(); // keep focus in the input
      this.accept(Number(li.dataset.i));
    });
  }

  private list: HTMLElement;
  private suggestions: Suggestion[] = [];
  private selected = 0;

  private refreshSuggestions() {
    const all = suggest(this.input.value);
    // Hide once the input already equals the only (finished) completion.
    const done = all.length === 1 && all[0].final && all[0].value === this.input.value.trim();
    this.showSuggestions(done ? [] : all.slice(0, 6));
  }

  private showSuggestions(list: Suggestion[]) {
    this.suggestions = list;
    this.selected = 0;
    this.list.hidden = list.length === 0;
    this.list.innerHTML = list
      .map(
        (s, i) =>
          `<li data-i="${i}" role="option" aria-selected="${i === 0}"><b>${escape(s.label)}</b><span>${escape(s.hint)}</span></li>`,
      )
      .join("");
  }

  private select(i: number) {
    const n = this.suggestions.length;
    this.selected = (i + n) % n;
    this.list.querySelectorAll("li").forEach((li, j) => li.setAttribute("aria-selected", String(j === this.selected)));
  }

  private accept(i: number) {
    const s = this.suggestions[i];
    if (!s) return;
    this.input.value = s.value;
    this.input.focus();
    this.refreshSuggestions();
  }

  /** ↑/↓ pick, Tab accepts, Enter accepts an unfinished completion (or sends), Esc closes. */
  private onKey(e: KeyboardEvent) {
    if (this.suggestions.length === 0) return;
    const current = this.suggestions[this.selected];
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      this.select(this.selected + (e.key === "ArrowDown" ? 1 : -1));
    } else if (e.key === "Tab") {
      e.preventDefault();
      this.accept(this.selected);
    } else if (e.key === "Enter" && completesWord(this.input.value, current.value)) {
      e.preventDefault();
      this.accept(this.selected);
    } else if (e.key === "Escape") {
      this.showSuggestions([]);
    }
  }

  update(pet: Pet) {
    for (const { key } of STATS) {
      const v = Math.round(pet[key]);
      const bar = this.bars.get(key)!;
      bar.fill.style.width = `${v}%`;
      bar.fill.dataset.level = v < 25 ? "bad" : v < 55 ? "warn" : "good";
      bar.value.textContent = String(v);
    }
    this.sleepBtn.textContent = pet.asleep ? "wake" : "sleep";
  }

  /** Puts the cursor in the chat box (which also keeps the panel open). */
  focusInput() {
    this.input.focus();
  }

  setThinking(thinking: boolean) {
    this.sendBtn.disabled = thinking;
  }
}

/**
 * Enter should finish a half-typed word (`/settings sk` → `skin`) but never tack on an
 * optional argument: `/personality` + Enter runs the command instead of becoming
 * `/personality reset`.
 */
function completesWord(typed: string, suggestion: string) {
  const t = typed.trim();
  const s = suggestion.trim();
  // A trailing space means "now pick the next part", e.g. `/settings ` + Enter → skin.
  return s !== t && (!s.startsWith(`${t} `) || typed.endsWith(" "));
}

function escape(s: string) {
  return s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);
}
