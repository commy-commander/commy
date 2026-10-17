const SERVER_COLORS = [
  ["#3b4fb8", "#161b3d"],
  ["#6a47b8", "#1f1840"],
  ["#1f7f8c", "#10253a"],
  ["#2f6db5", "#13203f"],
  ["#8a3f8f", "#2a1738"],
  ["#4a5a8f", "#171d38"],
  ["#a2507a", "#2c1832"],
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
