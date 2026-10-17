function tintHue(id) {
  let hash = 0;
  for (const ch of String(id)) hash = (hash * 31 + ch.charCodeAt(0)) | 0;
  return Math.abs(hash) % 360;
}

function personTint(userId) {
  return `hsl(${tintHue(userId)} 38% 44%)`;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { tintHue, personTint };
}
