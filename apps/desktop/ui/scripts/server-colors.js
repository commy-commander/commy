const SERVER_COLORS = [
  ["#4757d6", "#2f72c9"],
  ["#7a4fd6", "#a047b8"],
  ["#167f8c", "#2a6fc0"],
  ["#2f6fd0", "#5a50d0"],
  ["#9a45a8", "#c24a7e"],
  ["#2a78b8", "#1a828a"],
  ["#b8487a", "#7a4ac0"],
];

function serverColors(serverId) {
  let hash = 0;
  for (const ch of String(serverId)) hash = (hash * 31 + ch.charCodeAt(0)) | 0;
  const [from, to] = SERVER_COLORS[Math.abs(hash) % SERVER_COLORS.length];
  return { from, to, css: `linear-gradient(160deg, ${from}, ${to})` };
}

function serverTint(serverId) {
  return serverColors(serverId).from;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { SERVER_COLORS, serverColors, serverTint };
}
