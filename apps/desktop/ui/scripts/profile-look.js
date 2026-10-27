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

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    AVATAR_BORDERS, CARD_EFFECTS, BORDER_ORNAMENTS,
    isAvatarBorder, isCardEffect, ornamentPoints, stringSeed, particleSeeds,
  };
}
