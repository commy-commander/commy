function starSeed(id) {
  let hash = 2166136261;
  for (const ch of String(id)) hash = Math.imul(hash ^ ch.charCodeAt(0), 16777619);
  return hash >>> 0;
}

function starRandom(seed) {
  let s = seed || 1;
  return () => {
    s = (Math.imul(s, 1664525) + 1013904223) >>> 0;
    return s / 4294967296;
  };
}

const STAR_COUNT = 110;
const STAR_BASE = "linear-gradient(160deg, #12162a, #0b0d17)";
const STAR_HAZE = "radial-gradient(ellipse 70% 120% at 70% 0%, rgba(71, 87, 214, 0.07), transparent 70%)";

function starfield(id) {
  const rand = starRandom(starSeed(id));
  const layers = [];
  for (let i = 0; i < STAR_COUNT; i++) {
    const x = (rand() * 100).toFixed(1);
    const y = (rand() * 100).toFixed(1);
    const bright = rand() < 0.08;
    const size = bright ? 0.9 : 0.4 + rand() * 0.4;
    const alpha = (bright ? 0.5 : 0.12 + rand() * 0.25).toFixed(2);
    layers.push(`radial-gradient(circle at ${x}% ${y}%, rgba(242,239,230,${alpha}) 0, rgba(242,239,230,${alpha}) ${size.toFixed(2)}px, transparent ${(size + 0.5).toFixed(2)}px)`);
  }
  const stars = layers.join(", ");
  return { stars, css: `${stars}, ${STAR_HAZE}, ${STAR_BASE}` };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { starfield, starSeed };
}
