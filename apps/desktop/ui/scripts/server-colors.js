const SERVER_COLORS = [
  ["#4757d6", "#2f72c9"],
  ["#7a4fd6", "#a047b8"],
  ["#167f8c", "#2a6fc0"],
  ["#2f6fd0", "#5a50d0"],
  ["#9a45a8", "#c24a7e"],
  ["#2a78b8", "#1a828a"],
  ["#b8487a", "#7a4ac0"],
];

const STUDIO_SERVER_COLORS = ["#4a68ff", "#6f9fe6", "#58bfb2", "#8cc98b", "#e3b655", "#e0834a", "#f29bbd", "#b7a9f7"];
const SERVER_TINT_LIGHT = 0.25;

function serverColorIndex(serverId, count) {
  let hash = 0;
  for (const ch of String(serverId)) hash = (hash * 31 + ch.charCodeAt(0)) | 0;
  return Math.abs(hash) % count;
}

function serverColors(serverId) {
  const [from, to] = SERVER_COLORS[serverColorIndex(serverId, SERVER_COLORS.length)];
  const hexRgb = hex => { const n = parseInt(hex.slice(1), 16); return `${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`; };
  return { from, to, css: `linear-gradient(160deg, ${from}, ${to})`, glow: hexRgb(from) };
}

function serverLookNow() {
  return typeof studioLook === "function" && studioLook() ? "studio" : "classic";
}

function serverTint(serverId, look = serverLookNow()) {
  if (look === "studio") return STUDIO_SERVER_COLORS[serverColorIndex(serverId, STUDIO_SERVER_COLORS.length)];
  return serverColors(serverId).from;
}

function serverTintIsLight(color) {
  const hex = /^#([0-9a-f]{6})$/i.exec(String(color || "").trim());
  if (!hex) return false;
  const [r, g, b] = [0, 2, 4].map(at => {
    const c = parseInt(hex[1].slice(at, at + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b > SERVER_TINT_LIGHT;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { SERVER_COLORS, STUDIO_SERVER_COLORS, serverColors, serverTint, serverTintIsLight };
}
