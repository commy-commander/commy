const AVATAR_BORDERS = [
  { id: "ring", label: "Ring" },
  { id: "double", label: "Double" },
  { id: "sunset", label: "Sunset" },
  { id: "ocean", label: "Ocean" },
  { id: "neon", label: "Neon" },
  { id: "stars", label: "Stars" },
  { id: "leaves", label: "Leaves" },
  { id: "orbit", label: "Orbit" },
];
const CARD_EFFECTS = [
  { id: "sparkles", label: "Sparkles", particles: 14 },
  { id: "snow", label: "Snow", particles: 24 },
  { id: "aurora", label: "Aurora", particles: 0 },
  { id: "bubbles", label: "Bubbles", particles: 12 },
];
const BORDER_ORNAMENTS = {
  stars: { count: 5, from: 200, to: 340, size: 9 },
  leaves: { count: 6, from: 25, to: 155, size: 11 },
};

function isAvatarBorder(id) {
  return AVATAR_BORDERS.some(b => b.id === id);
}

function isCardEffect(id) {
  return CARD_EFFECTS.some(e => e.id === id);
}

function round2(n) {
  return Math.round(n * 100) / 100;
}

function ornamentPoints(count, fromDeg, toDeg) {
  const step = count > 1 ? (toDeg - fromDeg) / (count - 1) : 0;
  return Array.from({ length: count }, (_, i) => {
    const angle = fromDeg + step * i;
    const rad = (angle * Math.PI) / 180;
    return { x: round2(50 + 50 * Math.cos(rad)), y: round2(50 + 50 * Math.sin(rad)), angle: round2(angle + 90) };
  });
}

function stringSeed(text) {
  let hash = 2166136261;
  for (const ch of String(text || "")) {
    hash ^= ch.codePointAt(0);
    hash = Math.imul(hash, 16777619) >>> 0;
  }
  return hash;
}

function particleSeeds(count, seedText) {
  let state = stringSeed(seedText) || 1;
  const next = () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 4294967296;
  };
  return Array.from({ length: count }, () => ({
    x: round2(next() * 100),
    y: round2(next() * 100),
    delay: round2(next() * 4),
    size: round2(0.6 + next() * 0.8),
    duration: round2(3 + next() * 3),
  }));
}

const STUDIO_ORNAMENTS = {
  stars: { count: 5, from: 200, to: 340, turn: 0, path: "M5 .4 6.3 3.6 9.7 3.9 7.1 6.1 7.9 9.5 5 7.7 2.1 9.5 2.9 6.1 .3 3.9 3.7 3.6Z" },
  leaves: { count: 6, from: 25, to: 155, turn: 180, path: "M5 .3C8 2.4 8.4 6 5 9.7 1.6 6 2 2.4 5 .3Z" },
};
const STUDIO_SPARK = "M10 0C10.6 6.6 13.4 9.4 20 10 13.4 10.6 10.6 13.4 10 20 9.4 13.4 6.6 10.6 0 10 6.6 9.4 9.4 6.6 10 0Z";
const STUDIO_SPARKLES = [[52, 34, 30, 0, "hero"], [38, 21, 15, 1.4, ""], [64, 54, 13, 2.6, ""], [43, 57, 10, 0.7, ""], [61, 13, 8, 3.1, "sm"], [33, 43, 7, 1.9, "sm"], [90, 54, 7, 2.3, "sm"], [10, 42, 6, 0.3, "sm"], [86, 12, 5, 3.6, "sm"]];
const STUDIO_DUST = [[46, 12], [58, 44], [35, 33], [68, 30], [27, 60], [76, 62], [8, 16], [95, 30]];
const STUDIO_SNOW = [["far", 12, 3.2, 12], ["mid", 9, 5.4, 9], ["near", 4, 10, 6.5]];
const STUDIO_SNOW_SEED = 11;
const STUDIO_BUBBLES = [[52, 42, 30, 0, 9], [37, 63, 17, 2.2, 7.5], [64, 67, 12, 4.1, 6.5], [39, 21, 12, 1.1, 8], [63, 17, 8, 3.3, 7], [12, 36, 9, 5.2, 6.5], [89, 56, 10, 2.8, 8.5]];
const STUDIO_CURTAINS = [
  { cls: "c3", from: -4, to: 58, base: 16, amp: 2, wave: 0.16, phase: 0.6, height: 20, ink: "--accent-lit", peak: 0.7 },
  { cls: "c2", from: 14, to: 92, base: 20, amp: 2.6, wave: 0.13, phase: 2.1, height: 23, ink: "--tx-teal", peak: 0.8 },
  { cls: "c1", from: 34, to: 104, base: 24, amp: 3, wave: 0.11, phase: 4, height: 27, ink: "--tx-green", peak: 0.95 },
];

function studioOrnaments(id) {
  const kind = STUDIO_ORNAMENTS[id];
  if (!kind) return [];
  return ornamentPoints(kind.count, kind.from, kind.to).map((point, i) => ({ x: point.x, y: point.y, turn: round2(point.angle + kind.turn), odd: i % 2 === 1, path: kind.path }));
}

function studioSnow(count, seed) {
  let state = seed >>> 0;
  const next = () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 4294967296;
  };
  return Array.from({ length: count }, (_, k) => ({
    x: Math.round(((k + 0.15 + next() * 0.7) / count) * 1000) / 10,
    y: Math.round((4 + next() * 88) * 10) / 10,
    p: Math.round(next() * 1000) / 1000,
  }));
}

function studioCurtain(curtain) {
  const rays = [];
  for (let x = curtain.from; x < curtain.to; x += 1.1) {
    const t = (x - curtain.from) / (curtain.to - curtain.from);
    const env = Math.sin(Math.PI * t) ** 0.7;
    const hem = curtain.base + curtain.amp * Math.sin(x * curtain.wave + curtain.phase) + curtain.amp * 0.45 * Math.sin(x * curtain.wave * 2.3 + curtain.phase * 1.7);
    const n = 0.5 + 0.5 * Math.sin(x * 1.9 + curtain.phase * 3) * Math.sin(x * 0.73 + curtain.phase);
    const h = curtain.height * (0.62 + 0.38 * n);
    const o = env * (0.3 + 0.7 * (0.5 + 0.5 * Math.sin(x * 2.7 + curtain.phase * 5) * Math.cos(x * 0.41 + curtain.phase * 2)));
    rays.push({ x: round2(x), y: round2(hem - h), h: round2(h), o: round2(o) });
  }
  return rays;
}

function studioEffect(id) {
  if (id === "sparkles") {
    return [
      ...STUDIO_SPARKLES.map(([x, y, z, d, kind]) => ({ cls: `s${kind ? ` ${kind}` : ""}`, vars: { x, y, z, d }, path: STUDIO_SPARK })),
      ...STUDIO_DUST.map(([x, y], k) => ({ cls: "dust", vars: { x, y, d: Math.round(k * 7) / 10 } })),
    ];
  }
  if (id === "snow") {
    return STUDIO_SNOW.flatMap(([layer, count, z, t], i) => studioSnow(count, STUDIO_SNOW_SEED + i * 31).map(flake => ({ cls: layer, vars: { x: flake.x, y: flake.y, z, t, p: flake.p } })));
  }
  if (id === "bubbles") return STUDIO_BUBBLES.map(([x, y, z, d, t]) => ({ cls: "", vars: { x, y, z, d, t } }));
  return [];
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    AVATAR_BORDERS, CARD_EFFECTS, BORDER_ORNAMENTS,
    isAvatarBorder, isCardEffect, ornamentPoints, stringSeed, particleSeeds,
    STUDIO_ORNAMENTS, STUDIO_SPARKLES, STUDIO_DUST, STUDIO_SNOW, STUDIO_BUBBLES, STUDIO_CURTAINS,
    studioOrnaments, studioSnow, studioCurtain, studioEffect,
  };
}
