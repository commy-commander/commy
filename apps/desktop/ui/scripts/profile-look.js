const AVATAR_BORDERS = [
  { id: "ring", label: "Ring" },
  { id: "double", label: "Double" },
  { id: "sunset", label: "Sunset" },
  { id: "ocean", label: "Ocean" },
  { id: "neon", label: "Neon" },
  { id: "stars", label: "Stars" },
  { id: "leaves", label: "Leaves" },
  { id: "orbit", label: "Orbit" },
  { id: "halo", label: "Halo" },
  { id: "comet", label: "Comet" },
  { id: "eclipse", label: "Eclipse" },
  { id: "bloom", label: "Bloom" },
  { id: "frost", label: "Frost" },
  { id: "ripple", label: "Ripple" },
  { id: "prism", label: "Prism" },
  { id: "dial", label: "Dial" },
];
const CARD_EFFECTS = [
  { id: "sparkles", label: "Sparkles" },
  { id: "snow", label: "Snow" },
  { id: "aurora", label: "Aurora" },
  { id: "bubbles", label: "Bubbles" },
  { id: "fireflies", label: "Fireflies" },
  { id: "rain", label: "Rain" },
  { id: "meteors", label: "Meteors" },
  { id: "petals", label: "Petals" },
  { id: "sunbeams", label: "Sunbeams" },
  { id: "nebula", label: "Nebula" },
  { id: "confetti", label: "Confetti" },
];

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

const STUDIO_STAR = "M5 .4 6.3 3.6 9.7 3.9 7.1 6.1 7.9 9.5 5 7.7 2.1 9.5 2.9 6.1 .3 3.9 3.7 3.6Z";
const STUDIO_LEAF = "M5 .3C8 2.4 8.4 6 5 9.7 1.6 6 2 2.4 5 .3Z";
const STUDIO_ORNAMENTS = {
  stars: { count: 5, from: 200, to: 340, turn: 0, sizes: [0.8, 1, 1.3, 1, 0.8], path: STUDIO_STAR },
  leaves: { count: 6, from: 25, to: 155, turn: 180, sizes: [1], path: STUDIO_LEAF },
};
const STUDIO_FRAME_PARTS = {
  orbit: ["ab-orbit-dot"],
  comet: ["ab-head"],
  ripple: ["ab-wave", "ab-wave b"],
};
const STUDIO_SPARK = "M10 0C10.6 6.6 13.4 9.4 20 10 13.4 10.6 10.6 13.4 10 20 9.4 13.4 6.6 10.6 0 10 6.6 9.4 9.4 6.6 10 0Z";
const STUDIO_SPARKLES = [[52, 34, 30, 0, "hero"], [38, 21, 15, 1.4, ""], [64, 54, 13, 2.6, ""], [43, 57, 10, 0.7, ""], [61, 13, 8, 3.1, "sm"], [33, 43, 7, 1.9, "sm"], [90, 54, 7, 2.3, "sm"], [10, 42, 6, 0.3, "sm"], [86, 12, 5, 3.6, "sm"]];
const STUDIO_DUST = [[46, 12], [58, 44], [35, 33], [68, 30], [27, 60], [76, 62], [8, 16], [95, 30]];
const STUDIO_SNOW = [["far", 12, 3.2, 12], ["mid", 9, 5.4, 9], ["near", 4, 10, 6.5]];
const STUDIO_SNOW_SEED = 11;
const STUDIO_BUBBLES = [[52, 42, 30, 0, 9], [37, 63, 17, 2.2, 7.5], [64, 67, 12, 4.1, 6.5], [39, 21, 12, 1.1, 8], [63, 17, 8, 3.3, 7], [12, 36, 9, 5.2, 6.5], [89, 56, 10, 2.8, 8.5]];
const STUDIO_FIREFLIES = [[78, 22, 8.5, 0, 9], [62, 40, 6.5, 2.1, 11], [89, 48, 5.5, 4.4, 8], [69, 10, 5, 1.2, 12], [49, 24, 7, 3.3, 10], [37, 46, 5, 5.6, 13], [94, 16, 4.5, 6.1, 9], [24, 18, 5.5, 2.7, 12], [11, 40, 4, 4.9, 11], [56, 56, 4.5, 0.8, 10]];
const STUDIO_RAIN = [["far", 14, 12, 1.6], ["mid", 11, 18, 1.25], ["near", 6, 27, 0.95]];
const STUDIO_RAIN_SEED = 23;
const STUDIO_NIGHT = [[8, 14], [19, 40], [27, 9], [36, 30], [45, 16], [55, 44], [63, 8], [71, 28], [82, 48], [90, 12], [96, 36], [50, 58]];
const STUDIO_METEORS = [[88, 2, 54, 0.7, 4.6], [106, 24, 42, 2.9, 6.2], [62, -4, 36, 5.6, 7.4], [118, 8, 46, 1.6, 5.4], [40, -6, 30, 4.2, 8.2]];
const STUDIO_PETAL = "M10 1.5C14.8 5.2 16 11.5 10 18.5 4 11.5 5.2 5.2 10 1.5Z";
const STUDIO_PETALS = [["far", 5, 8, 14], ["mid", 5, 12.5, 10], ["near", 2, 18, 7.5]];
const STUDIO_PETALS_SEED = 5;
const STUDIO_MOTES = [[70, 20], [58, 36], [80, 44], [44, 18], [66, 54], [88, 28], [36, 40]];
const STUDIO_CLOUDS = [["n1", 70, 26, 110, 0, 19], ["n2", 36, 10, 95, 6, 23], ["n3", 92, 56, 85, 11, 17], ["n4", 12, 44, 75, 3, 21]];
const STUDIO_CONFETTI = { count: 20, seed: 41, inks: 6 };
const STUDIO_CURTAINS = [
  { cls: "c3", from: -4, to: 58, base: 16, amp: 2, wave: 0.16, phase: 0.6, height: 20, ink: "--accent-lit", peak: 0.7 },
  { cls: "c2", from: 14, to: 92, base: 20, amp: 2.6, wave: 0.13, phase: 2.1, height: 23, ink: "--tx-teal", peak: 0.8 },
  { cls: "c1", from: 34, to: 104, base: 24, amp: 3, wave: 0.11, phase: 4, height: 27, ink: "--tx-green", peak: 0.95 },
];

function studioOrnaments(id) {
  const kind = STUDIO_ORNAMENTS[id];
  if (!kind) return [];
  return ornamentPoints(kind.count, kind.from, kind.to).map((point, i) => ({ x: point.x, y: point.y, turn: round2(point.angle + kind.turn), odd: i % 2 === 1, size: kind.sizes[i % kind.sizes.length], path: kind.path }));
}

const STUDIO_BLOOM = { petals: 16, seat: [40.5, 1.9], back: [40.2, 51.6, 5.4, 11.8], front: [40.2, 48.4, 4.4, 9.4], shine: [42.6, 46.6, 0.6, 1.9], bead: [42.9, 0.85] };
const STUDIO_FROST = { ring: [42.5, 2], line: 46.4, points: 6, tall: [41.8, 0, 10.8, 2.7], core: [43.7, 0, 6.3, 1.1], side: [42.9, 52, 7.4, 1.55], small: [43.8, 0, 5.4, 1.5] };
const STUDIO_DIAL = { rim: 49.4, seat: 42.2, minor: [46.7, 48.4], hour: [44.3, 48.5], quarter: [43.6, 48.5], hand: [49.1, 42.9, 3.3] };

function polar(radius, deg) {
  const rad = ((deg - 90) * Math.PI) / 180;
  return `${round2(50 + radius * Math.cos(rad))} ${round2(50 + radius * Math.sin(rad))}`;
}

function bloomPetal(deg, [base, tip, foot, width]) {
  const shoulder = base + (tip - base) * 0.3;
  return `M${polar(base, deg - foot)}C${polar(shoulder, deg - width)} ${polar(tip, deg - width)} ${polar(tip, deg)}C${polar(tip, deg + width)} ${polar(shoulder, deg + width)} ${polar(base, deg + foot)}Z`;
}

function frostShard(deg, [from, tilt, length, half]) {
  const root = ((deg - 90) * Math.PI) / 180;
  const lean = ((deg + tilt - 90) * Math.PI) / 180;
  const at = (along, across) => `${round2(50 + from * Math.cos(root) + along * Math.cos(lean) - across * Math.sin(lean))} ${round2(50 + from * Math.sin(root) + along * Math.sin(lean) + across * Math.cos(lean))}`;
  return `M${at(0, 0)}L${at(length * 0.36, -half)}L${at(length, 0)}L${at(length * 0.36, half)}Z`;
}

function spokes(count, [from, to], skip) {
  let d = "";
  for (let k = 0; k < count; k++) {
    const deg = (k * 360) / count;
    if (skip && deg % skip === 0) continue;
    d += `M${polar(from, deg)}L${polar(to, deg)}`;
  }
  return d;
}

function studioFrameArt(id) {
  if (id === "bloom") {
    const { petals, seat, back, front, shine, bead } = STUDIO_BLOOM;
    const step = 360 / petals;
    const turns = Array.from({ length: petals }, (_, k) => k * step);
    const ring = (cls, shape, turn) => `<g class="${cls}">${turns.map(deg => `<path d="${bloomPetal(deg + turn, shape)}"/>`).join("")}</g>`;
    return `<g class="bl-wreath">${ring("bl-back", back, 0)}${ring("bl-front", front, step / 2)}${ring("bl-shine", shine, step / 2)}<g class="bl-bead">${turns.map(deg => { const [x, y] = polar(bead[0], deg).split(" "); return `<circle cx="${x}" cy="${y}" r="${bead[1]}"/>`; }).join("")}</g></g><circle class="bl-seat" cx="50" cy="50" r="${seat[0]}" stroke-width="${seat[1]}"/>`;
  }
  if (id === "frost") {
    const { ring, line, points, tall, core, side, small } = STUDIO_FROST;
    const step = 360 / points;
    const turns = Array.from({ length: points }, (_, k) => k * step);
    const shards = (cls, draw) => `<path class="${cls}" d="${turns.map(draw).join("")}"/>`;
    const circle = (cls, radius, width) => `<circle class="${cls}" cx="50" cy="50" r="${round2(radius)}"${width ? ` stroke-width="${width}"` : ""}/>`;
    const [from, tilt, length, half] = side;
    return `${circle("fr-line", line)}${shards("fr-small", deg => frostShard(deg + step / 2, small))}${shards("fr-side", deg => frostShard(deg, [from, -tilt, length, half]) + frostShard(deg, side))}${circle("fr-ring", ring[0], ring[1])}${circle("fr-edge", ring[0] - ring[1] / 2)}${circle("fr-edge", ring[0] + ring[1] / 2)}${shards("fr-tall", deg => frostShard(deg, tall))}${shards("fr-core", deg => frostShard(deg, core))}`;
  }
  if (id === "dial") {
    const { rim, seat, minor, hour, quarter, hand } = STUDIO_DIAL;
    const [edge, point, spread] = hand;
    return `<circle class="di-band" cx="50" cy="50" r="${round2((rim + seat) / 2)}" stroke-width="${round2(rim - seat)}"/><circle class="di-rim" cx="50" cy="50" r="${rim}"/><circle class="di-seat" cx="50" cy="50" r="${seat}"/><path class="di-minor" d="${spokes(60, minor, 30)}"/><path class="di-hour" d="${spokes(12, hour, 90)}"/><path class="di-quarter" d="${spokes(4, quarter)}"/><path class="di-hand" d="M${polar(edge, -spread)}L${polar(edge, spread)}L${polar(point, 0)}Z"/>`;
  }
  return "";
}

function studioFrame(id) {
  return [
    ...studioOrnaments(id).map((mark, i) => ({ cls: mark.odd ? "ab-o b" : "ab-o", vars: { x: `${mark.x}%`, y: `${mark.y}%`, r: `${mark.turn}deg`, z: mark.size, d: round2(i * 0.7) }, path: mark.path })),
    ...(STUDIO_FRAME_PARTS[id] || []).map(cls => ({ cls })),
  ];
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
  if (id === "fireflies") return STUDIO_FIREFLIES.map(([x, y, z, d, t], k) => ({ cls: k % 2 ? "b" : "", vars: { x, y, z, d, t } }));
  if (id === "rain") {
    return STUDIO_RAIN.flatMap(([layer, count, z, t], i) => studioSnow(count, STUDIO_RAIN_SEED + i * 17).map(drop => ({ cls: layer, vars: { x: drop.x, y: drop.y, z, t, p: drop.p } })));
  }
  if (id === "meteors") {
    return [
      ...studioNight(),
      ...STUDIO_METEORS.map(([x, y, z, d, t]) => ({ cls: "m", vars: { x, y, z, d, t } })),
    ];
  }
  if (id === "petals") {
    return STUDIO_PETALS.flatMap(([layer, count, z, t], i) => studioSnow(count, STUDIO_PETALS_SEED + i * 13).map((petal, k) => ({ cls: k % 2 ? `${layer} b` : layer, vars: { x: petal.x, y: petal.y, z, t, p: petal.p }, path: STUDIO_PETAL })));
  }
  if (id === "sunbeams") {
    return [
      { cls: "beam" },
      { cls: "beam b" },
      ...STUDIO_MOTES.map(([x, y], k) => ({ cls: "dust", vars: { x, y, d: Math.round(k * 9) / 10 } })),
    ];
  }
  if (id === "nebula") {
    return [
      ...STUDIO_CLOUDS.map(([ink, x, y, z, d, t]) => ({ cls: `cloud ${ink}`, vars: { x, y, z, d, t } })),
      ...studioNight(),
    ];
  }
  if (id === "confetti") {
    return studioSnow(STUDIO_CONFETTI.count, STUDIO_CONFETTI.seed).map((bit, k) => ({ cls: `k${(k % STUDIO_CONFETTI.inks) + 1}`, vars: { x: bit.x, y: bit.y, z: round2(6.5 + ((k * 7) % 5) * 0.7), t: round2(5.2 + ((k * 3) % 7) * 0.4), p: bit.p } }));
  }
  return [];
}

function studioNight() {
  return STUDIO_NIGHT.map(([x, y], k) => ({ cls: k % 3 ? "dust" : "dust lg", vars: { x, y, d: Math.round(k * 6) / 10 } }));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    AVATAR_BORDERS, CARD_EFFECTS,
    isAvatarBorder, isCardEffect, ornamentPoints,
    STUDIO_ORNAMENTS, STUDIO_FRAME_PARTS, STUDIO_BLOOM, STUDIO_FROST, STUDIO_DIAL, STUDIO_SPARKLES, STUDIO_DUST, STUDIO_SNOW, STUDIO_BUBBLES, STUDIO_CURTAINS,
    STUDIO_FIREFLIES, STUDIO_RAIN, STUDIO_NIGHT, STUDIO_METEORS, STUDIO_PETALS, STUDIO_MOTES, STUDIO_CLOUDS, STUDIO_CONFETTI,
    studioOrnaments, studioFrame, studioFrameArt, studioSnow, studioCurtain, studioEffect,
  };
}
