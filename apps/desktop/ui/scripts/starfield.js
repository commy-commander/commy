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

function starfield(id, glow) {
  const rand = starRandom(starSeed(id));
  const layers = [];
  const count = 16;
  for (let i = 0; i < count; i++) {
    const x = Math.round(rand() * 100);
    const y = Math.round(rand() * 100);
    const big = rand() < 0.22;
    const size = big ? 1.4 : 0.9;
    const alpha = (big ? 0.85 : 0.35 + rand() * 0.3).toFixed(2);
    layers.push(`radial-gradient(circle at ${x}% ${y}%, rgba(242,239,230,${alpha}) 0, rgba(242,239,230,${alpha}) ${size}px, transparent ${size + 0.6}px)`);
  }
  const gx = Math.round(15 + rand() * 70);
  const gy = Math.round(rand() * 60);
  const rgb = glow || "71, 87, 214";
  const stars = layers.join(", ");
  const nebula = `radial-gradient(ellipse 70% 120% at ${gx}% ${gy}%, rgba(${rgb}, 0.5), transparent 70%)`;
  return {
    stars,
    css: `${stars}, ${nebula}, linear-gradient(160deg, #151929, #0b0d17)`,
  };
}

function hueGlow(hue) {
  const a = 0.55 * Math.min(0.5, 1 - 0.5);
  const f = n => {
    const k = (n + hue / 30) % 12;
    return Math.round((0.5 - a * Math.max(-1, Math.min(k - 3, 9 - k, 1))) * 255);
  };
  return `${f(0)}, ${f(8)}, ${f(4)}`;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { starfield, starSeed, hueGlow };
}
