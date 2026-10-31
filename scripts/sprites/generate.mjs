// Generates the pet sprite sheets: one PNG atlas per skin (rows = animations,
// columns = frames) plus src/sprites.json describing the layout.
//
//   node scripts/sprites/generate.mjs            # all skins
//   node scripts/sprites/generate.mjs ember      # just one
//
// Frames are drawn procedurally as SVG, then rendered by headless Chrome/Edge
// (set CHROME=path to override). Intermediate SVG sheets go to design/sprites/.

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { SKINS } from "./skins.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const F = 128; // frame size in SVG units / CSS px
const SCALE = 2; // PNG pixels per CSS px, for HiDPI screens
const COLS = 8;
const GROUND = 112;
const INK = "#2B1B17";

const sin = (i, n, phase = 0) => Math.sin(((i + phase) / n) * Math.PI * 2);

/**
 * Each animation is a list of frame parameters:
 * sx/sy squash-stretch, tx/ty offset, rot degrees, eyes/mouth face parts,
 * a = "perkiness" -1..1 (drives sprouts, ears, rays), t = 0..1 loop phase,
 * fx = overlay effects.
 */
const ANIMS = {
  // Resting loops stay calm: a faint breath, long holds, and the occasional blink or hop.
  // `durations` (ms per frame) overrides fps where a frame should linger.
  idle: { fps: 2, loop: true, durations: [900, 900, 900, 900, 900, 900, 900, 130], frames: range(8, (i) => ({
    sx: 1 + 0.008 * Math.sin((i / 7) * Math.PI * 2), sy: 1 - 0.008 * Math.sin((i / 7) * Math.PI * 2),
    eyes: i === 7 ? "blink" : "open", mouth: "smile", a: 0.3, t: 0,
  })) },
  happy: { fps: 9, loop: true, durations: [3500, 110, 110, 110, 110, 110, 110, 900], frames: [
    { sx: 1, sy: 1, eyes: "open", mouth: "grin", a: 1, t: 0 },
    { sx: 1.08, sy: 0.9, eyes: "happy", mouth: "grin", a: 0.8, t: 0.12 },
    { sx: 0.95, sy: 1.06, ty: -8, eyes: "happy", mouth: "grin", a: 1, t: 0.25 },
    { sx: 0.97, sy: 1.04, ty: -12, eyes: "happy", mouth: "grin", a: 1, t: 0.37 },
    { sx: 0.98, sy: 1.03, ty: -8, eyes: "happy", mouth: "grin", a: 1, t: 0.5 },
    { sx: 1.07, sy: 0.92, eyes: "happy", mouth: "grin", a: 0.8, t: 0.62 },
    { sx: 1, sy: 1, eyes: "open", mouth: "grin", a: 1, t: 0.75 },
    { sx: 1, sy: 1, eyes: "open", mouth: "smile", a: 1, t: 0.87 },
  ] },
  sad: { fps: 4, loop: true, frames: range(6, (i, n) => ({
    sx: 1 + 0.015 * sin(i, n), sy: 0.97 - 0.015 * sin(i, n),
    eyes: "sad", mouth: "frown", a: -0.9, t: i / n, fx: i >= 2 && i <= 4 ? { tear: (i - 2) / 2 } : {},
  })) },
  hungry: { fps: 8, loop: true, durations: [1200, 1200, 1200, 600, 90, 90, 90, 600], frames: range(8, (i) => ({
    tx: [0, 0, 0, 0, -2, 2, -1.5, 1][i], sx: 1, sy: 0.99,
    eyes: "open", mouth: i >= 4 ? "open" : "openSmall", a: -0.4, t: i / 8,
  })) },
  tired: { fps: 4, loop: true, frames: range(6, (i, n) => ({
    rot: 2.5 * sin(i, n), sx: 1.02, sy: 0.97, eyes: i === 3 ? "closed" : "half",
    mouth: "flat", a: -0.7, t: i / n,
  })) },
  sleep: { fps: 4, loop: true, frames: range(8, (i, n) => ({
    sx: 1.03 + 0.03 * sin(i, n), sy: 0.97 - 0.03 * sin(i, n),
    eyes: "closed", mouth: "flat", a: -1, t: i / n, fx: { zzz: i / n },
  })) },
  eat: { fps: 8, loop: false, frames: range(8, (i) => ({
    sx: i % 2 ? 1.04 : 0.98, sy: i % 2 ? 0.96 : 1.02,
    eyes: i % 2 ? "happy" : "open", mouth: i % 2 ? "smile" : "open", a: 0.8, t: i / 8,
    fx: i >= 2 ? { crumbs: (i - 2) / 5 } : {},
  })) },
  play: { fps: 10, loop: false, frames: [
    { sx: 1.1, sy: 0.88, eyes: "happy", mouth: "grin", a: 1, t: 0 },
    { sx: 0.92, sy: 1.1, ty: -10, eyes: "happy", mouth: "grin", a: 1, t: 0.12 },
    { sx: 0.95, sy: 1.06, ty: -16, rot: -8, eyes: "happy", mouth: "grin", a: 1, t: 0.25, fx: { heart: 0 } },
    { sx: 0.97, sy: 1.03, ty: -18, rot: 8, eyes: "happy", mouth: "grin", a: 1, t: 0.37, fx: { heart: 0.3 } },
    { sx: 0.95, sy: 1.06, ty: -10, eyes: "open", mouth: "grin", a: 1, t: 0.5, fx: { heart: 0.6 } },
    { sx: 1.1, sy: 0.88, eyes: "happy", mouth: "grin", a: 1, t: 0.62, fx: { heart: 0.9 } },
    { sx: 0.98, sy: 1.02, eyes: "open", mouth: "grin", a: 1, t: 0.75 },
    { sx: 1, sy: 1, eyes: "open", mouth: "smile", a: 1, t: 0.87 },
  ] },
  // Eight mouth shapes in a speech-like rhythm, with a small bob and one blink.
  talk: { fps: 10, loop: true, frames: range(8, (i) => ({
    sx: 1,
    sy: [1, 1.02, 1.01, 1, 1.02, 1, 1.01, 1][i],
    ty: [0, -1, 0, 0, -1, 0, 0, 0][i],
    eyes: i === 5 ? "blink" : "open",
    mouth: ["talkWide", "talkMid", "openSmall", "talkWide", "talkMid", "flat", "talkWide", "openSmall"][i],
    a: 0.5,
    t: i / 8,
  })) },
  wiggle: { fps: 10, loop: false, frames: [-9, 8, -6, 4, -2, 0].map((rot, i) => ({
    rot, sx: 1, sy: 1, eyes: i < 4 ? "happy" : "open", mouth: "grin", a: 0.9, t: i / 6, fx: { heart: i / 5 },
  })) },
  // Quitting: a little wave, eyes go x_x, it flops into a puddle and its soul floats away.
  bye: { fps: 3, loop: false, durations: [380, 380, 520, 420, 360, 360, 360, 700], frames: [
    { rot: -5, eyes: "happy", mouth: "grin", a: 0.6, t: 0 },
    { rot: 5, eyes: "happy", mouth: "grin", a: 0.6, t: 0.2 },
    { sx: 1.02, sy: 0.98, eyes: "closed", mouth: "flat", a: -0.3, t: 0.4 },
    { sx: 1.1, sy: 0.86, eyes: "dead", mouth: "openSmall", a: -1, t: 0.5 },
    { sx: 1.16, sy: 0.78, eyes: "dead", mouth: "openSmall", a: -1, t: 0.5, fx: { soul: 0.1 } },
    { sx: 1.2, sy: 0.72, fade: 0.75, eyes: "dead", mouth: "flat", a: -1, t: 0.5, fx: { soul: 0.4 } },
    { sx: 1.22, sy: 0.7, fade: 0.45, eyes: "dead", mouth: "flat", a: -1, t: 0.5, fx: { soul: 0.7 } },
    { sx: 1.24, sy: 0.68, fade: 0.15, eyes: "dead", mouth: "flat", a: -1, t: 0.5, fx: { soul: 1 } },
  ] },
};

function range(n, f) {
  return Array.from({ length: n }, (_, i) => f(i, n));
}

// ---------- shared face + effects ----------

function eye(kind, x, y, side) {
  const s = `stroke="${INK}" stroke-width="2.8" stroke-linecap="round" fill="none"`;
  switch (kind) {
    case "open":
      return `<ellipse cx="${x}" cy="${y}" rx="4.6" ry="6" fill="${INK}"/><circle cx="${x + 1.6}" cy="${y - 2.2}" r="1.7" fill="#fff"/>`;
    case "blink":
      return `<path d="M${x - 4.5} ${y + 1}h9" ${s}/>`;
    case "closed":
      return `<path d="M${x - 5} ${y}q5 4.5 10 0" ${s}/>`;
    case "half":
      return `<path d="M${x - 4.6} ${y}a4.6 4 0 0 0 9.2 0z" fill="${INK}"/><path d="M${x - 5.5} ${y}h11" ${s}/>`;
    case "happy":
      return `<path d="M${x - 5} ${y + 2}q5 -7 10 0" ${s}/>`;
    case "dead":
      return `<path d="M${x - 4} ${y - 4}l8 8M${x + 4} ${y - 4}l-8 8" ${s}/>`;
    case "sad": {
      const brow = side < 0 ? `M${x - 5} ${y - 9}L${x + 4} ${y - 12}` : `M${x + 5} ${y - 9}L${x - 4} ${y - 12}`;
      return `<ellipse cx="${x}" cy="${y + 1}" rx="4.4" ry="5.4" fill="${INK}"/><circle cx="${x + 1.4}" cy="${y - 1}" r="1.5" fill="#fff"/><path d="${brow}" ${s}/>`;
    }
  }
}

function mouth(kind, y, cat) {
  const s = `stroke="${INK}" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round" fill="none"`;
  switch (kind) {
    case "smile":
      return cat ? `<path d="M57 ${y}q3.5 4.5 7 0q3.5 4.5 7 0" ${s}/>` : `<path d="M58 ${y}q6 6 12 0" ${s}/>`;
    case "grin":
      return `<path d="M57 ${y - 1}q7 10 14 0z" fill="${INK}" stroke="${INK}" stroke-width="1.5" stroke-linejoin="round"/><path d="M60 ${y + 3.2}q4 2.6 8 0" stroke="#E5566B" stroke-width="2.2" stroke-linecap="round" fill="none"/>`;
    case "flat":
      return `<path d="M59.5 ${y + 1}h9" ${s}/>`;
    case "frown":
      return `<path d="M58 ${y + 3}q6 -5.5 12 0" ${s}/>`;
    case "open":
      return `<ellipse cx="64" cy="${y + 1}" rx="4.2" ry="4" fill="${INK}"/>`;
    case "openSmall":
      return `<ellipse cx="64" cy="${y + 1}" rx="3" ry="2.2" fill="${INK}"/>`;
    // Talking: a big "aah" with a tongue and a medium "eh", so the mouth clearly moves.
    case "talkWide":
      return `<ellipse cx="64" cy="${y + 1.6}" rx="5.8" ry="5.6" fill="${INK}"/><ellipse cx="64" cy="${y + 4.6}" rx="3.3" ry="1.9" fill="#E5566B"/>`;
    case "talkMid":
      return `<ellipse cx="64" cy="${y + 1.2}" rx="5" ry="3.4" fill="${INK}"/><ellipse cx="64" cy="${y + 3}" rx="2.6" ry="1.1" fill="#E5566B"/>`;
  }
}

function face(p, skin) {
  let out = faceParts(p, skin);
  // Dark-bodied skins can recolour the face (eyes, mouth) so it stays visible.
  if (skin.face.ink) out = out.replaceAll(INK, skin.face.ink);
  // Optional details drawn over the face, e.g. slit pupils.
  if (skin.overFace) out += skin.overFace(p);
  return skin.faceTransform ? `<g transform="${skin.faceTransform(p)}">${out}</g>` : out;
}

function faceParts(p, skin) {
  const { eyeY, eyeDX, mouthY, cheek = "#F7A48E", cat, grumpy } = skin.face;
  if (grumpy) {
    // Unimpressed: flat brows over open eyes, and a lopsided smirk instead of a smile.
    const brows = p.eyes === "open" || p.eyes === "blink"
      ? `<path d="M${64 - eyeDX - 6} ${eyeY - 8}h11M${64 + eyeDX - 5} ${eyeY - 9}l11 1.5" stroke="${INK}" stroke-width="2.8" stroke-linecap="round"/>`
      : "";
    const m = p.mouth === "smile"
      ? `<path d="M58 ${mouthY + 1}q7 2.5 12 -3" stroke="${INK}" stroke-width="2.8" stroke-linecap="round" fill="none"/>`
      : mouth(p.mouth, mouthY);
    return eye(p.eyes, 64 - eyeDX, eyeY, -1) + eye(p.eyes, 64 + eyeDX, eyeY, 1) + brows + m;
  }
  const cheeks = `<ellipse cx="${64 - eyeDX - 9}" cy="${eyeY + 9}" rx="5.5" ry="3.2" fill="${cheek}" opacity="${p.eyes === "happy" ? 0.95 : 0.7}"/>
    <ellipse cx="${64 + eyeDX + 9}" cy="${eyeY + 9}" rx="5.5" ry="3.2" fill="${cheek}" opacity="${p.eyes === "happy" ? 0.95 : 0.7}"/>`;
  return cheeks + eye(p.eyes, 64 - eyeDX, eyeY, -1) + eye(p.eyes, 64 + eyeDX, eyeY, 1) + mouth(p.mouth, mouthY, cat);
}

const HEART = "M0 4c-3-5-11-4-11 2 0 5 11 11 11 11s11-6 11-11c0-6-8-7-11-2z";

function effects(fx = {}, skin) {
  let out = "";
  if (fx.zzz !== undefined) {
    for (const [k, off] of [[0, 0], [1, 0.5]]) {
      const ph = (fx.zzz + off) % 1;
      const op = ph < 0.25 ? ph / 0.25 : 1 - (ph - 0.25) / 0.75;
      const size = 11 + k * 3;
      out += `<text x="${96 + ph * 10 + k * 6}" y="${36 - ph * 22 - k * 4}" font-family="Segoe UI, Arial, sans-serif" font-weight="800" font-size="${size}" fill="#8A7168" opacity="${op.toFixed(2)}">z</text>`;
    }
  }
  if (fx.heart !== undefined) {
    const ph = fx.heart;
    const op = ph < 0.3 ? ph / 0.3 : 1 - (ph - 0.3) / 0.7;
    out += `<path d="${HEART}" fill="#E5566B" opacity="${Math.max(0.15, op).toFixed(2)}" transform="translate(${92} ${24 - ph * 14}) scale(${0.6 + 0.3 * Math.min(1, ph * 3)})"/>`;
  }
  if (fx.crumbs !== undefined) {
    const ph = fx.crumbs;
    const y = skin.face.mouthY + 6;
    for (const [dx, dy, r] of [[-9, 0, 1.8], [8, 3, 1.5], [-3, 6, 1.3]]) {
      out += `<circle cx="${64 + dx * (1 + ph)}" cy="${y + dy + ph * 18}" r="${r}" fill="#E8C27A" opacity="${(1 - ph * 0.8).toFixed(2)}"/>`;
    }
  }
  if (fx.tear !== undefined) {
    const { eyeY, eyeDX } = skin.face;
    const y = eyeY + 7 + fx.tear * 10;
    out += `<path d="M${64 - eyeDX - 3} ${y}q-2.6 4 0 6q2.6 -2 0 -6z" fill="#7FB6E8" opacity="${(1 - fx.tear * 0.5).toFixed(2)}"/>`;
  }
  if (fx.soul !== undefined) {
    // A little ghost with a peaceful face, drifting up and to the side with a slight sway.
    const ph = fx.soul;
    const x = 64 + Math.sin(ph * Math.PI * 1.5) * 8;
    const y = 64 - ph * 40;
    const op = ph < 0.15 ? 0.6 + ph * 2.5 : 0.95 - Math.max(0, ph - 0.6) * 1.8;
    const s = 1 + ph * 0.3;
    out += `<g transform="translate(${x.toFixed(1)} ${y.toFixed(1)}) scale(${s.toFixed(2)}) rotate(${(Math.sin(ph * 9) * 6).toFixed(1)})" opacity="${Math.max(0, op).toFixed(2)}">
      <path d="M-12 4c0-10 5-17 12-17s12 7 12 17v11l-4-3-4 3-4-3-4 3-4-3-4 3z" fill="#FFFFFF" stroke="#C9DDF0" stroke-width="1.5" stroke-linejoin="round"/>
      <path d="M-6 0q2 2 4 0M2 0q2 2 4 0" stroke="${INK}" stroke-width="1.6" stroke-linecap="round" fill="none"/>
      <ellipse cx="-8" cy="4" rx="2.4" ry="1.4" fill="#F7A48E" opacity=".7"/><ellipse cx="8" cy="4" rx="2.4" ry="1.4" fill="#F7A48E" opacity=".7"/>
    </g>`;
  }
  return out;
}

// ---------- assembly ----------

function frameSvg(p, skin) {
  const { sx = 1, sy = 1, tx = 0, ty = 0, rot = 0, fade = 1 } = p;
  const fit = skin.fit ?? 1; // shrinks tall skins so jumps stay inside the cell
  const lift = Math.min(1, -ty / 30);
  const shadow = `<ellipse cx="64" cy="${GROUND + 3}" rx="${(34 * (1 - lift * 0.45) * sx).toFixed(1)}" ry="${(4.5 * (1 - lift * 0.4)).toFixed(1)}" fill="#2B1B17" opacity="${(0.18 * (1 - lift * 0.5)).toFixed(2)}"/>`;
  const body = skin.draw(p) + face(p, skin);
  const t = `translate(${64 + tx} ${GROUND + ty}) rotate(${rot}) scale(${(sx * fit).toFixed(3)} ${(sy * fit).toFixed(3)}) translate(-64 -${GROUND})`;
  return `<g opacity="${fade}">${shadow}<g transform="${t}">${body}</g></g>${effects(p.fx, skin)}`;
}

function sheetSvg(skin) {
  const names = Object.keys(ANIMS);
  const w = COLS * F;
  const h = names.length * F;
  let frames = "";
  names.forEach((name, row) => {
    ANIMS[name].frames.forEach((p, col) => {
      frames += `<g transform="translate(${col * F} ${row * F})" clip-path="url(#cell)">${frameSvg(p, skin)}</g>\n`;
    });
  });
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">
<defs><clipPath id="cell"><rect width="${F}" height="${F}"/></clipPath>${skin.defs ?? ""}</defs>
${frames}</svg>`;
}

function findChrome() {
  const candidates = [
    process.env.CHROME,
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
    "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
  ].filter(Boolean);
  const found = candidates.find((c) => existsSync(c));
  if (!found) throw new Error("No Chrome/Edge found; set CHROME=/path/to/chrome");
  return found;
}

function renderPng(svgPath, pngPath, w, h) {
  const html = svgPath.replace(/\.svg$/, ".html");
  writeFileSync(
    html,
    `<!doctype html><html><body style="margin:0;background:transparent"><img src="${pathToFileURL(svgPath)}" width="${w}" height="${h}" style="display:block"></body></html>`,
  );
  execFileSync(findChrome(), [
    "--headless=new",
    "--disable-gpu",
    "--hide-scrollbars",
    `--force-device-scale-factor=${SCALE}`,
    `--window-size=${w},${h}`,
    "--default-background-color=00000000",
    `--screenshot=${pngPath}`,
    pathToFileURL(html).href,
  ], { stdio: "ignore" });
}

const only = process.argv.slice(2);
const svgDir = join(ROOT, "design/sprites");
const pngDir = join(ROOT, "public/sprites");
mkdirSync(svgDir, { recursive: true });
mkdirSync(pngDir, { recursive: true });

const names = Object.keys(ANIMS);
for (const [id, skin] of Object.entries(SKINS)) {
  if (only.length && !only.includes(id)) continue;
  const svgPath = join(svgDir, `${id}.svg`);
  writeFileSync(svgPath, sheetSvg(skin));
  renderPng(svgPath, join(pngDir, `${id}.png`), COLS * F, names.length * F);
  console.log(`wrote public/sprites/${id}.png`);
}

const meta = {
  frame: F,
  cols: COLS,
  rows: names.length,
  skins: Object.fromEntries(Object.entries(SKINS).map(([id, s]) => [id, s.label])),
  anims: Object.fromEntries(
    names.map((name, row) => {
      const { frames, fps, loop, durations } = ANIMS[name];
      return [name, { row, frames: frames.length, loop, durations: durations ?? frames.map(() => Math.round(1000 / fps)) }];
    }),
  ),
};
writeFileSync(join(ROOT, "src/sprites.json"), JSON.stringify(meta, null, 2) + "\n");
console.log("wrote src/sprites.json");
