// Generated demo art for screenshots. Everything is drawn here, no stock images.
// Avatars are characters in the mascot's family; the game and the shared
// picture are pixel art so the demo community, Pixel Pioneers, looks like one.

const SKINS = [
  ["#ff6b6b", "#ffd5d5"], ["#6b8bff", "#dbe3ff"], ["#2fd3a7", "#d2f7ec"], ["#ffcf4d", "#fff1c2"],
  ["#b07cff", "#e9dcff"], ["#4dd0ff", "#d0f1ff"], ["#ff8a4d", "#ffe0cf"], ["#5ee0c0", "#dcf7f0"],
  ["#9aa3b5", "#e2e5ec"],
];

// Face shared by every character: the mascot's eyes and smile, simplified.
function face(x, y, s, ink = "#1b1b1f") {
  return `<g transform="translate(${x} ${y}) scale(${s})">
<circle cx="-16" cy="0" r="7" fill="${ink}"/><circle cx="16" cy="0" r="7" fill="${ink}"/>
<circle cx="-13.5" cy="-2.5" r="2.4" fill="#fff"/><circle cx="18.5" cy="-2.5" r="2.4" fill="#fff"/>
<path d="M-14 14 Q0 26 14 14" fill="none" stroke="${ink}" stroke-width="5" stroke-linecap="round"/>
</g>`;
}

// Hats and accessories, one per character, so the row of avatars reads as people.
const TOPS = [
  // beanie
  (c) => `<path d="M34 54 Q64 18 94 54 L94 62 L34 62 Z" fill="${c}"/><rect x="30" y="56" width="68" height="12" rx="6" fill="${c}"/><circle cx="64" cy="22" r="7" fill="${c}"/>`,
  // headphones
  (c) => `<path d="M34 70 Q34 28 64 28 Q94 28 94 70" fill="none" stroke="${c}" stroke-width="7"/><rect x="26" y="62" width="14" height="24" rx="6" fill="${c}"/><rect x="88" y="62" width="14" height="24" rx="6" fill="${c}"/>`,
  // cat ears
  (c) => `<path d="M38 60 L34 26 L62 44 Z" fill="${c}"/><path d="M90 60 L94 26 L66 44 Z" fill="${c}"/>`,
  // crown
  (c) => `<path d="M38 56 L38 32 L52 44 L64 26 L76 44 L90 32 L90 56 Z" fill="${c}"/>`,
  // antenna
  (c) => `<rect x="61" y="18" width="6" height="26" fill="${c}"/><circle cx="64" cy="16" r="8" fill="${c}"/>`,
  // cap
  (c) => `<path d="M32 58 Q64 20 96 58 Z" fill="${c}"/><rect x="60" y="54" width="46" height="9" rx="4" fill="${c}"/>`,
  // helmet glass
  (c) => `<circle cx="64" cy="66" r="44" fill="none" stroke="${c}" stroke-width="6"/><path d="M36 50 Q44 32 62 28" fill="none" stroke="#fff" stroke-width="4" stroke-linecap="round" opacity=".7"/>`,
  // flower
  (c) => `<g transform="translate(92 40)"><circle cx="-8" cy="0" r="6" fill="${c}"/><circle cx="8" cy="0" r="6" fill="${c}"/><circle cx="0" cy="-8" r="6" fill="${c}"/><circle cx="0" cy="8" r="6" fill="${c}"/><circle cx="0" cy="0" r="5" fill="#fff"/></g>`,
  // glasses
  (c) => `<g fill="none" stroke="${c}" stroke-width="5"><circle cx="48" cy="68" r="13"/><circle cx="80" cy="68" r="13"/><path d="M61 68 H67"/></g>`,
];

export function avatarSvg(i) {
  const [tone, pale] = SKINS[i % SKINS.length];
  const hat = TOPS[i % TOPS.length]("#1b1b1f");
  const ink = "#1b1b1f";
  return `<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128">
<rect width="128" height="128" fill="${tone}"/>
<ellipse cx="64" cy="138" rx="46" ry="40" fill="${pale}"/>
<circle cx="64" cy="70" r="40" fill="#fff"/>
${face(64, 68, 1, ink)}
<circle cx="44" cy="80" r="5" fill="${tone}" opacity=".55"/><circle cx="84" cy="80" r="5" fill="${tone}" opacity=".55"/>
${hat}
</svg>`;
}

export function serverIconSvg() {
  // Pixel Pioneers: a pixel-art planet with a flag, 16 by 16 cells.
  const P = { ".": null, b: "#2b1d5c", p: "#7c5cff", d: "#5a3ddb", r: "#ff4d4d", w: "#ffffff", y: "#ffd84d" };
  const rows = [
    "................",
    "......pppp......",
    "....pppppppp....",
    "...pppdpppppp...",
    "..ppppppppdppp..",
    "..ppdppppppppp..",
    ".pppppppdpppppp.",
    ".pppppprppppppp.",
    ".pppdprrrppppppy",
    ".ppppppppdppppp.",
    "..pppppppppppp..",
    "..pppdpppppppp..",
    "...ppppppppdpw..",
    "....pppppppp....",
    "......pppp......",
    "................",
  ];
  return pixelSvg(rows, P, 16, "#2b1d5c", 256);
}

function pixelSvg(rows, palette, cell, bg, size) {
  const w = rows[0].length;
  const h = rows.length;
  const rects = [];
  rows.forEach((row, y) => {
    for (let x = 0; x < row.length; x++) {
      const c = palette[row[x]];
      if (c) rects.push(`<rect x="${x * cell}" y="${y * cell}" width="${cell}" height="${cell}" fill="${c}"/>`);
    }
  });
  const scale = size ? size / (w * cell) : 1;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${w * cell * scale}" height="${h * cell * scale}" viewBox="0 0 ${w * cell} ${h * cell}" shape-rendering="crispEdges">
<rect width="${w * cell}" height="${h * cell}" fill="${bg}"/>${rects.join("")}</svg>`;
}

// A small deterministic generator so the scenes are the same on every run.
function rng(seed) {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 4294967296;
  };
}

// Sprites, in cells. Each is a list of strings; "." is transparent.
const SPRITES = {
  tree: {
    pal: { t: "#6b3f1f", l: "#2f8f4a", d: "#227038", h: "#4fbf6a" },
    rows: ["...hh...", "..hllh..", ".hlllld.", ".llllld.", "lllldddd", ".lldddd.", "...tt...", "...tt..."],
  },
  house: {
    pal: { r: "#c24a3b", d: "#8f3328", w: "#f0e3c8", k: "#5a3a26", g: "#ffd96b", o: "#3a2a20" },
    rows: [
      "....rrrr....", "...rrrrrr...", "..rrrdrrrr..", ".rrrrrrrdrr.", "rrrrrrrrrrrr", "oooooooooooo",
      "owwwwwwwwwwo", "owggwwwwggwo", "owggwwkkggwo", "owwwwwkkwwwo", "owwwwwkkwwwo", "oooooooooooo",
    ],
  },
  hero: {
    pal: { s: "#ffd2b0", h: "#3b2a20", r: "#ff4d4d", b: "#3f5bd6", k: "#1b1b1f" },
    rows: ["..hhhh..", ".hhhhhh.", ".sskssk.", ".ssssss.", "..rrrr..", ".rrrrrr.", "..bbbb..", "..k..k.."],
  },
  coin: { pal: { y: "#ffd84d", o: "#e0a92a" }, rows: [".yy.", "yyoy", "yoyy", ".yy."] },
  cloud: { pal: { w: "#ffffff" }, rows: ["...wwww....", ".wwwwwwwww.", "wwwwwwwwwww", ".wwwwwwwww."] },
  heart: { pal: { r: "#ff4d4d" }, rows: [".r.r.", "rrrrr", ".rrr.", "..r.."] },
  sheep: { pal: { w: "#ffffff", k: "#2a2a30" }, rows: ["..wwww..", ".wwwwww.", "kwwwwwwk", ".wwwwww.", "..k..k.."] },
  flower: { pal: { p: "#ff7ab8", y: "#ffd84d", g: "#2f8f4a" }, rows: [".p.", "pyp", ".p.", ".g."] },
};

function sprite(name, x, y, cell, flip = false) {
  const { pal, rows } = SPRITES[name];
  const out = [];
  rows.forEach((row, ry) => {
    for (let rx = 0; rx < row.length; rx++) {
      const c = pal[row[rx]];
      if (!c) continue;
      const cx = flip ? row.length - 1 - rx : rx;
      out.push(`<rect x="${x + cx * cell}" y="${y + ry * cell}" width="${cell}" height="${cell}" fill="${c}"/>`);
    }
  });
  return out.join("");
}

// A cosy pixel-art farm game frame for the screen share demo, 1280 by 720.
export function gameSvg() {
  const W = 1280;
  const H = 720;
  const c = 8;
  const r = rng(7);
  const parts = [];
  // sky and hills
  parts.push(`<rect width="${W}" height="${H}" fill="#7ec8f0"/>`);
  parts.push(`<rect y="200" width="${W}" height="${H}" fill="#a6ddf7"/>`);
  for (let i = 0; i < 6; i++) parts.push(sprite("cloud", 40 + i * 230 + (i % 2) * 60, 40 + (i % 3) * 36, c));
  parts.push(`<path d="M0 330 Q160 250 320 320 T640 320 T960 316 T1280 324 L1280 420 L0 420 Z" fill="#5fae5a"/>`);
  parts.push(`<path d="M0 370 Q200 300 420 360 T840 356 T1280 364 L1280 440 L0 440 Z" fill="#4c9a4b"/>`);
  // ground: grass tiles with variation, then a tilled field and a path
  for (let y = 400; y < H; y += c * 2) {
    for (let x = 0; x < W; x += c * 2) {
      const n = r();
      const fill = n < 0.08 ? "#3f8a40" : n < 0.14 ? "#5bb257" : "#4aa14a";
      parts.push(`<rect x="${x}" y="${y}" width="${c * 2}" height="${c * 2}" fill="${fill}"/>`);
    }
  }
  for (let y = 520; y < 680; y += c * 2) {
    for (let x = 120; x < 560; x += c * 2) {
      const row = Math.floor((y - 520) / (c * 2));
      parts.push(`<rect x="${x}" y="${y}" width="${c * 2}" height="${c * 2}" fill="${row % 2 ? "#8a5a34" : "#6f4628"}"/>`);
    }
  }
  for (let y = 520; y < 680; y += c * 4) {
    for (let x = 128; x < 560; x += c * 4) parts.push(`<rect x="${x}" y="${y + 2}" width="8" height="12" fill="#5ee06a"/><rect x="${x - 4}" y="${y + 6}" width="16" height="4" fill="#3fa84a"/>`);
  }
  parts.push(`<path d="M620 420 Q700 520 640 620 T700 720 L780 720 Q720 620 760 520 T700 420 Z" fill="#d9b77a"/>`);
  parts.push(`<path d="M640 430 Q710 520 660 610 T720 710" fill="none" stroke="#c7a263" stroke-width="10"/>`);
  // trees, house, animals, flowers, coins, hero
  [[20, 330], [980, 300], [1100, 330], [1190, 420], [860, 440], [60, 460]].forEach(([x, y]) => parts.push(sprite("tree", x, y, c)));
  parts.push(sprite("house", 880, 470, 10));
  parts.push(sprite("sheep", 1010, 600, 6));
  parts.push(sprite("sheep", 1070, 630, 6, true));
  for (let i = 0; i < 9; i++) parts.push(sprite("flower", 60 + i * 60 + (i % 3) * 10, 700 - (i % 2) * 24, 5));
  [[560, 470], [600, 452], [640, 470], [820, 560], [840, 600]].forEach(([x, y]) => parts.push(sprite("coin", x, y, 6)));
  parts.push(sprite("hero", 668, 548, 8));
  // speech bubble
  parts.push(`<rect x="700" y="492" width="150" height="44" rx="4" fill="#fff"/><path d="M712 536 L704 548 L728 536 Z" fill="#fff"/>`);
  parts.push(`<text x="712" y="521" font-family="Verdana, sans-serif" font-size="18" font-weight="700" fill="#1b1b1f">market day!</text>`);
  // HUD
  for (let i = 0; i < 3; i++) parts.push(sprite("heart", 32 + i * 34, 28, 6));
  parts.push(sprite("coin", 32, 70, 6));
  parts.push(`<g font-family="Verdana, sans-serif" font-weight="700" fill="#fff" stroke="#1b1b1f" stroke-width="4" paint-order="stroke">
<text x="64" y="92" font-size="24">248</text>
<text x="${W - 32}" y="52" font-size="22" text-anchor="end">DAY 14  SPRING</text>
<text x="${W - 32}" y="84" font-size="22" text-anchor="end">10:40 AM</text></g>`);
  // energy bar
  parts.push(`<rect x="${W - 60}" y="${H - 220}" width="28" height="180" fill="#1b1b1f"/><rect x="${W - 54}" y="${H - 150}" width="16" height="104" fill="#5ee06a"/>`);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" shape-rendering="crispEdges">${parts.join("")}</svg>`;
}

// "Look what we built": a pixel-art castle at dusk, shared in chat, 800 by 450.
export function islandSvg() {
  const W = 800;
  const H = 450;
  const c = 8;
  const r = rng(3);
  const parts = [`<rect width="${W}" height="${H}" fill="#1b1f4a"/>`, `<rect y="150" width="${W}" height="${H}" fill="#3a2d6b"/>`, `<rect y="230" width="${W}" height="${H}" fill="#7b3f6e"/>`];
  for (let i = 0; i < 70; i++) parts.push(`<rect x="${Math.floor(r() * 100) * c}" y="${Math.floor(r() * 22) * c}" width="${c / 2}" height="${c / 2}" fill="#fff" opacity="${0.5 + r() * 0.5}"/>`);
  parts.push(`<rect x="600" y="60" width="56" height="56" fill="#fff3c4"/><rect x="616" y="52" width="24" height="8" fill="#fff3c4"/><rect x="616" y="116" width="24" height="8" fill="#fff3c4"/><rect x="592" y="76" width="8" height="24" fill="#fff3c4"/><rect x="656" y="76" width="8" height="24" fill="#fff3c4"/>`);
  // water
  for (let y = 330; y < H; y += c) {
    for (let x = 0; x < W; x += c) {
      const n = r();
      parts.push(`<rect x="${x}" y="${y}" width="${c}" height="${c}" fill="${n < 0.06 ? "#5aa7e6" : n < 0.1 ? "#2f6fb8" : "#2a5aa8"}"/>`);
    }
  }
  // island
  parts.push(`<rect x="200" y="300" width="400" height="40" fill="#5c9a4a"/><rect x="176" y="316" width="448" height="24" fill="#4c7f3c"/><rect x="160" y="332" width="480" height="16" fill="#8d6a3d"/>`);
  // castle: walls, towers, gate, flags
  const stone = (x, y, w, h) => {
    const out = [];
    for (let yy = y; yy < y + h; yy += c) for (let xx = x; xx < x + w; xx += c) out.push(`<rect x="${xx}" y="${yy}" width="${c}" height="${c}" fill="${r() < 0.18 ? "#8b93a7" : "#a7aec0"}"/>`);
    return out.join("");
  };
  parts.push(stone(248, 212, 304, 88));
  parts.push(stone(224, 150, 64, 150));
  parts.push(stone(512, 150, 64, 150));
  parts.push(stone(368, 120, 64, 180));
  for (const x of [224, 240, 256, 272, 512, 528, 544, 560, 368, 384, 400, 416]) parts.push(`<rect x="${x}" y="${x < 300 || x > 500 ? 142 : 112}" width="8" height="8" fill="#a7aec0"/>`);
  for (let x = 304; x < 512; x += 16) parts.push(`<rect x="${x}" y="204" width="8" height="8" fill="#a7aec0"/>`);
  parts.push(`<rect x="384" y="252" width="32" height="48" fill="#3a2614"/><rect x="384" y="244" width="32" height="8" fill="#3a2614"/>`);
  for (const [x, y] of [[248, 180], [536, 180], [392, 150], [392, 174]]) parts.push(`<rect x="${x}" y="${y}" width="8" height="16" fill="#ffd96b"/>`);
  for (const x of [256, 544, 400]) parts.push(`<rect x="${x}" y="${x === 400 ? 88 : 118}" width="4" height="32" fill="#3a2614"/><rect x="${x + 4}" y="${x === 400 ? 88 : 118}" width="16" height="10" fill="#ff4d4d"/>`);
  // trees and a lantern path
  parts.push(sprite("tree", 180, 250, 6));
  parts.push(sprite("tree", 580, 246, 6));
  for (let x = 344; x <= 440; x += 32) parts.push(`<rect x="${x}" y="312" width="8" height="8" fill="#ffd96b"/>`);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" shape-rendering="crispEdges">${parts.join("")}</svg>`;
}

export function svgDataUrl(svg) {
  return "data:image/svg+xml;base64," + Buffer.from(svg).toString("base64");
}
