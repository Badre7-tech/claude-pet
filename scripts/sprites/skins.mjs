// Pet skins for the sprite generator. Each skin draws its body for one frame in a
// 128×128 box standing on y = 112; the shared face is drawn on top at `face`.
// p.a is "perkiness" (-1 droopy … 1 perky) and p.t the 0..1 loop phase.

const wave = (t, k = 1) => Math.sin(t * Math.PI * 2 * k);
const deg = (r) => (r * Math.PI) / 180;

function gradient(id, light, dark) {
  return `<radialGradient id="${id}" cx="40%" cy="32%" r="72%"><stop offset="0" stop-color="${light}"/><stop offset="1" stop-color="${dark}"/></radialGradient>`;
}

const BONE = "#C9BB9F";
const WING = "#E4D9C2";

/**
 * Builds a grumpy-spine skin. Posture tracks mood: upright when perky, slumped when low.
 * `segs` go bottom → top as { y, w, h }; `head` is the top bone that carries the face.
 */
function spineSkin({ label, segs, head, face }) {
  const n = segs.length;
  const pivot = head.y + head.h; // the head tilts around its base
  const bend = (p) => ({ slouch: (1 - p.a) * 4.5, sway: wave(p.t) * 1.2 });
  // Sideways offset at height `k` (0 = sacrum … n = head): one smooth curve.
  const offset = (k, { slouch, sway }) => {
    const up = k / n;
    return -slouch * 1.3 * up * up + sway * up;
  };
  const headTransform = (p) => {
    const b = bend(p);
    return `translate(${offset(n + 0.2, b).toFixed(1)} 0) rotate(${(-b.slouch * 1.3 + b.sway).toFixed(1)} 64 ${pivot})`;
  };

  const headSvg = `<rect x="${64 - head.w / 2}" y="${head.y}" width="${head.w}" height="${head.h}" rx="${head.rx}" fill="url(#bone-g)" stroke="${BONE}" stroke-width="1.2"/>`;

  return {
    label,
    defs: `${gradient("bone-g", "#FBF6EA", "#D9CDB4")}
      <linearGradient id="disc-g" x1="0" x2="0" y1="0" y2="1"><stop offset="0" stop-color="#EDA7A0"/><stop offset="1" stop-color="#D98C86"/></linearGradient>`,
    face: { cheek: "#E9B9A8", grumpy: true, ...face },
    draw: (p) => {
      const b = bend(p);
      let out = `<path d="M51 101h26q-3 10-13 12q-10-2-13-12z" fill="url(#bone-g)" stroke="${BONE}" stroke-width="1"/>
        <path d="M58 104h12M60 108h8" stroke="${BONE}" stroke-width="1.4" stroke-linecap="round"/>`;
      segs.forEach((s, i) => {
        const x = 64 + offset(i + 0.5, b);
        const top = s.y - s.h / 2;
        const wl = x - s.w / 2;
        const wr = x + s.w / 2;
        const wing = Math.max(5, s.w * 0.28);
        out += `<rect x="${(wl + 3).toFixed(1)}" y="${(s.y + s.h / 2 - 0.6).toFixed(1)}" width="${s.w - 6}" height="${Math.max(2.4, s.h * 0.3).toFixed(1)}" rx="1.3" fill="url(#disc-g)"/>`;
        out += `<path d="M${(wl + 2).toFixed(1)} ${(top + 1.5).toFixed(1)}L${(wl - wing).toFixed(1)} ${(s.y + 1.5).toFixed(1)}L${(wl + 2).toFixed(1)} ${(s.y + s.h * 0.35).toFixed(1)}z
          M${(wr - 2).toFixed(1)} ${(top + 1.5).toFixed(1)}L${(wr + wing).toFixed(1)} ${(s.y + 1.5).toFixed(1)}L${(wr - 2).toFixed(1)} ${(s.y + s.h * 0.35).toFixed(1)}z" fill="${WING}" stroke="${BONE}" stroke-width="1" stroke-linejoin="round"/>`;
        out += `<rect x="${wl.toFixed(1)}" y="${top.toFixed(1)}" width="${s.w}" height="${s.h}" rx="${(s.h / 2.2).toFixed(1)}" fill="url(#bone-g)" stroke="${BONE}" stroke-width="1"/>`;
      });
      return `${out}<g transform="${headTransform(p)}">${headSvg}
        <path d="M${64 - head.w / 2 + 6} ${head.y + 8}q5-4 12-4" stroke="#fff" stroke-width="2.6" stroke-linecap="round" fill="none" opacity=".7"/></g>`;
    },
    faceTransform: headTransform,
  };
}

/** Evenly stacked vertebrae from just above the sacrum up to `topY`, tapering in width. */
function column(count, topY, wBottom, wTop, hBottom, hTop) {
  const bottomY = 96;
  return Array.from({ length: count }, (_, i) => {
    const t = count === 1 ? 0 : i / (count - 1);
    return {
      y: bottomY - (bottomY - topY) * t,
      w: wBottom + (wTop - wBottom) * t,
      h: hBottom + (hTop - hBottom) * t,
    };
  });
}

const STAR = (cx, cy, r) => {
  const pts = Array.from({ length: 10 }, (_, i) => {
    const ang = deg(-90 + i * 36);
    const rr = i % 2 ? r * 0.45 : r;
    return `${(cx + Math.cos(ang) * rr).toFixed(1)},${(cy + Math.sin(ang) * rr).toFixed(1)}`;
  });
  return `M${pts.join("L")}Z`;
};

export const SKINS = {
  blob: {
    label: "Blob",
    defs: gradient("blob-g", "#F4A27E", "#D9643F"),
    face: { eyeY: 68, eyeDX: 16, mouthY: 83 },
    draw: () => `
      <ellipse cx="46" cy="108" rx="10" ry="6" fill="#C4552F"/>
      <ellipse cx="82" cy="108" rx="10" ry="6" fill="#C4552F"/>
      <path d="M64 24c28 0 48 22 48 50 0 26-21 40-48 40S16 100 16 74c0-28 20-50 48-50z" fill="url(#blob-g)"/>`,
  },

  sprout: {
    label: "Sprout",
    defs: gradient("sprout-g", "#FBE3CF", "#EDB896"),
    face: { eyeY: 80, eyeDX: 15, mouthY: 92 },
    // The sprout is the mood antenna: upright when perky, drooping when sad, sways in idle.
    draw: (p) => {
      const droop = (1 - p.a) * 32;
      const sway = wave(p.t) * 3 + (p.a < 0 ? droop * 0.35 : 0);
      return `
      <g transform="rotate(${sway.toFixed(1)} 64 47)">
        <path d="M64 47V29" stroke="#5E9A50" stroke-width="4.5" stroke-linecap="round"/>
        <path d="M64 31c-3-9-12-13-21-11 2 9 11 14 21 11z" fill="#7DB46C" transform="rotate(${(-droop).toFixed(1)} 64 31)"/>
        <path d="M65 28c4-11 15-15 25-12-2 11-14 16-25 12z" fill="#8CC57A" transform="rotate(${droop.toFixed(1)} 65 28)"/>
      </g>
      <path d="M18 98c0-32 20-53 46-53s46 21 46 53c0 9-6 14-15 14H33c-9 0-15-5-15-14z" fill="url(#sprout-g)"/>
      <path d="M30 70c4-10 12-17 22-20" stroke="#fff" stroke-width="4" stroke-linecap="round" fill="none" opacity=".55"/>`;
    },
  },

  beancat: {
    label: "Bean Cat",
    defs: gradient("cat-g", "#F7C88A", "#E0913E"),
    face: { eyeY: 68, eyeDX: 12, mouthY: 80, cat: true },
    // Ears flatten when sad; the tail swishes with the loop phase.
    draw: (p) => {
      const ear = (1 - p.a) * 9;
      const tail = wave(p.t) * 9;
      return `
      <path d="M95 97c15 1 21-17 10-26" stroke="#D07F30" stroke-width="7" stroke-linecap="round" fill="none" transform="rotate(${tail.toFixed(1)} 95 97)"/>
      <g fill="#E0913E" stroke="#E0913E" stroke-width="6" stroke-linejoin="round">
        <path d="M37 50 40 22l19 13z" transform="rotate(${(-ear).toFixed(1)} 44 48)"/>
        <path d="M91 50l-3-28-19 13z" transform="rotate(${ear.toFixed(1)} 84 48)"/>
      </g>
      <path d="M41 45l1.6-14 9 6.4zM87 45l-1.6-14-9 6.4z" fill="#F7A48E" opacity=".8"/>
      <path d="M32 59c0-18 14-28 32-28s32 10 32 28v32c0 14-13 21-32 21s-32-7-32-21z" fill="url(#cat-g)"/>
      <path d="M58 33l6 7 6-7" stroke="#D07F30" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" fill="none" opacity=".6"/>`;
    },
  },

  ember: {
    label: "Ember",
    fit: 0.88,
    defs: `${gradient("ember-g", "#FFC46B", "#F0642A")}
      <radialGradient id="ember-core" cx="50%" cy="60%" r="50%"><stop offset="0" stop-color="#FFE7A8" stop-opacity=".9"/><stop offset="1" stop-color="#FFE7A8" stop-opacity="0"/></radialGradient>`,
    face: { eyeY: 82, eyeDX: 11, mouthY: 94, cheek: "#FF9A7A" },
    // The tip flickers with the phase and leans more when perky; the inner glow dims when low.
    draw: (p) => {
      const flick = wave(p.t, 2) * 2.5 + p.a * 2;
      const glow = 0.35 + 0.5 * ((p.a + 1) / 2);
      return `
      <path d="M64 112c-24 0-38-15-38-36 0-19 12-31 24-43 7-7 ${11 + flick * 0.3}-15 ${9 + flick}-24 17 5 29 19 32 34 7 8 11 20 11 33 0 21-14 36-38 36z" fill="url(#ember-g)"/>
      <ellipse cx="64" cy="86" rx="26" ry="22" fill="url(#ember-core)" opacity="${glow.toFixed(2)}"/>`;
    },
  },

  // Pyro: a majestic black-and-gold baby dragon — glowing gold eyes with slit pupils,
  // curved golden horns, a ridge of scales, clawed wings and a smooth tail with a gold fin.
  // Spits golden sparks while eating.
  pyro: {
    label: "Pyro",
    fit: 0.8,
    defs: `${gradient("pyro-g", "#4A4356", "#0D0B11")}
      <linearGradient id="pyro-horn" x1="1" y1="1" x2="0" y2="0"><stop offset="0" stop-color="#A97A22"/><stop offset=".6" stop-color="#E8BC4E"/><stop offset="1" stop-color="#FFF0B8"/></linearGradient>
      <linearGradient id="pyro-wing" x1="0" y1="1" x2="1" y2="0"><stop offset="0" stop-color="#17141C"/><stop offset=".6" stop-color="#3A2F2A"/><stop offset="1" stop-color="#8A6A28"/></linearGradient>
      <linearGradient id="pyro-belly" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#FFE08A"/><stop offset="1" stop-color="#C9922E"/></linearGradient>
      <radialGradient id="pyro-aura" cx="50%" cy="50%" r="50%"><stop offset="0" stop-color="#FFD866" stop-opacity=".55"/><stop offset=".6" stop-color="#F5B83A" stop-opacity=".18"/><stop offset="1" stop-color="#F5B83A" stop-opacity="0"/></radialGradient>`,
    face: { eyeY: 64, eyeDX: 14, mouthY: 84, cheek: "#B0566A", ink: "#F5C542" },
    // Dragon eyes: a dark slit pupil on the glowing gold eye, plus a fresh glint.
    overFace: (p) => {
      if (p.eyes !== "open" && p.eyes !== "sad") return "";
      const dy = p.eyes === "sad" ? 1.2 : 0.3;
      return [50, 78]
        .map((x) => `<ellipse cx="${x + 0.2}" cy="${64 + dy}" rx="1.3" ry="4.2" fill="#1A1206"/><circle cx="${x + 2}" cy="${64 + dy - 2.8}" r="1.2" fill="#fff"/>`)
        .join("");
    },
    draw: (p) => {
      const perk = (p.a + 1) / 2;
      const flap = wave(p.t, 2) * 11 * (0.25 + perk);
      const droop = (1 - p.a) * 12;
      const wingAngle = (-flap + droop).toFixed(1);
      const swish = (wave(p.t) * 7).toFixed(1);
      const GOLD = "#E8BC4E";
      const DEEP = "#9C7424";
      const wingSvg = `
        <path d="M92 62C96 44 104 30 112 20L117 12L116 21C121 25 125 30 127 37C122 38 120 42 122 48C117 47 114 51 115 57C111 56 107 59 106 64C102 63 98 67 97 73Z" fill="url(#pyro-wing)" stroke="${DEEP}" stroke-width="1.8" stroke-linejoin="round"/>
        <path d="M93 63L113 20M113 21L126 36M113 21L121 47M113 21L114 56" stroke="${GOLD}" stroke-width="1.8" stroke-linecap="round" opacity=".9"/>
        <path d="M113 21L117 12L116 21z" fill="#FFE9A8"/>`;
      const horn = `
        <path d="M44 41Q33 27 30 9Q43 19 57 35Z" fill="url(#pyro-horn)" stroke="#8F6A1E" stroke-width="1.2" stroke-linejoin="round"/>
        <path d="M38.1 32.2Q43.5 30.5 48.7 25.9M34.3 24.4Q38.3 23 41.9 19.2M32 17.8Q34.5 17 36.6 14.4" stroke="${DEEP}" stroke-width="1.2" stroke-linecap="round" fill="none" opacity=".65"/>
        <path d="M41 37Q34 26 31.5 13" stroke="#FFF6D0" stroke-width="1.2" stroke-linecap="round" fill="none" opacity=".6"/>`;
      let sparks = "";
      if (p.fx?.crumbs !== undefined) {
        const ph = p.fx.crumbs;
        sparks = [[-1, 0.9], [1, 0.7], [0.2, 1.2], [-0.4, 1.4]]
          .map(([dir, sp]) => `<path d="${STAR(64 + dir * (6 + ph * 24 * sp), 88 - ph * 18 * sp, 4.2 - ph * 1.8)}" fill="${dir > 0 ? "#FFD34D" : "#FFB13D"}" opacity="${(1 - ph * 0.6).toFixed(2)}"/>`)
          .join("");
      }
      const brows = p.eyes === "sad" || p.eyes === "dead" ? "" : `<path d="M41 53q7 -5 14 -2M87 53q-7 -5 -14 -2" stroke="${GOLD}" stroke-width="2.2" stroke-linecap="round" fill="none" opacity=".9"/>`;
      // Tiny arms resting on the belly, each with two golden claws.
      const arm = (side) => `
        <g transform="translate(${64 + side * 26} 90) rotate(${side * -28})">
          <ellipse cx="0" cy="0" rx="5.5" ry="9" fill="#1F1A25" stroke="#B8902F" stroke-width="1" stroke-opacity=".6"/>
          <path d="M-2.4 8.2q1 2.8 2 0zM0.6 8.4q1 2.8 2 0z" fill="#F2C54E"/>
        </g>`;
      // A soft golden aura when it's really happy.
      const aura = p.a >= 0.8 ? `<circle cx="64" cy="70" r="58" fill="url(#pyro-aura)"/>` : "";
      return `
      ${aura}
      <g transform="rotate(${swish} 88 100)">
        <path d="M84 98C102 110 121 102 122 82C122 76 119 71 116 68C118 80 114 92 102 96C96 98 90 97 84 94Z" fill="url(#pyro-g)" stroke="#B8902F" stroke-width="1" stroke-opacity=".6"/>
        <path d="M88 101C103 108 118 100 119.5 84" stroke="${GOLD}" stroke-width="1.6" stroke-linecap="round" fill="none" opacity=".75"/>
        <path d="M116.5 69C113 64 114 58 119 54C120 59 121 64 119.5 70Z" fill="${GOLD}" stroke="#8F6A1E" stroke-width="1" stroke-linejoin="round"/>
      </g>
      <g transform="rotate(${wingAngle} 92 62)">${wingSvg}</g>
      <g transform="translate(128 0) scale(-1 1)"><g transform="rotate(${wingAngle} 92 62)">${wingSvg}</g></g>
      ${horn}
      <g transform="translate(128 0) scale(-1 1)">${horn}</g>
      <path d="M58 33L60.5 26L63 31.5L64 22L65 31.5L67.5 26L70 33Z" fill="${GOLD}" stroke="#8F6A1E" stroke-width="1" stroke-linejoin="round"/>
      <ellipse cx="46" cy="108" rx="10" ry="6" fill="#16131B" stroke="#B8902F" stroke-width="1" stroke-opacity=".55"/>
      <ellipse cx="82" cy="108" rx="10" ry="6" fill="#16131B" stroke="#B8902F" stroke-width="1" stroke-opacity=".55"/>
      <path d="M64 30c25 0 42 20 42 45 0 24-18 37-42 37S22 99 22 75c0-25 17-45 42-45z" fill="url(#pyro-g)" stroke="#B8902F" stroke-width="1.3" stroke-opacity=".55"/>
      <path d="M30 86q4-3.5 8 0M33 95q4-3.5 8 0M90 86q4-3.5 8 0M87 95q4-3.5 8 0M36 76q4-3.5 8 0M84 76q4-3.5 8 0" stroke="${GOLD}" stroke-width="1.3" stroke-linecap="round" fill="none" opacity=".22"/>
      <path d="M45 93c4 11 11 18 19 18s15-7 19-18c-6 4-12 5-19 5s-13-1-19-5z" fill="url(#pyro-belly)"/>
      <path d="M48 100h32M51 105h26M55 109h18" stroke="#8F6A1E" stroke-width="1.3" stroke-linecap="round" opacity=".55"/>
      ${arm(-1)}${arm(1)}
      ${brows}
      <path d="M34 60c3-12 11-21 22-25" stroke="#fff" stroke-width="4" stroke-linecap="round" fill="none" opacity=".16"/>
      ${sparks}`;
    },
  },

  // A grumpy human spine: a tall column of vertebrae, the top one just big enough for a face.
  spine: spineSkin({
    label: "Spine",
    segs: column(7, 50, 30, 22, 7.5, 6.5),
    head: { y: 18, w: 34, h: 26, rx: 10 },
    face: { eyeY: 31, eyeDX: 8, mouthY: 39 },
  }),
};
