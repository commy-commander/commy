const CALL_TILE_RATIO = 16 / 9;
const PHONE_TILES_PER_PAGE = 4;

function streamOpenAction({ userId, meId, sharing, watchingUser }) {
  if (userId && userId === meId && sharing) return "preview";
  if (userId && userId === watchingUser) return "already";
  return "watch";
}

function callTileKey(kind, user) {
  return `${kind}:${user}`;
}

function callTiles({ userIds, names, cameras, shares, meId, channelId }) {
  const here = new Map((userIds || []).map((id, i) => [id, (names || [])[i] || ""]));
  const sharing = (shares || []).filter(s => here.has(s.user) && (!channelId || !s.channel || s.channel === channelId));
  const others = list => list.filter(t => !t.me);
  const mine = list => list.filter(t => t.me);
  const screens = sharing.map(s => ({ key: callTileKey("screen", s.user), kind: "screen", user: s.user, name: here.get(s.user) || s.name, me: s.user === meId }));
  const cams = [...new Set(cameras || [])]
    .filter(id => here.has(id))
    .map(id => ({ key: callTileKey("camera", id), kind: "camera", user: id, name: here.get(id), me: id === meId }));
  return [...others(screens), ...mine(screens), ...mine(cams), ...others(cams)];
}

function callGridSize(count, width, height, gap = 8) {
  if (!(width > 0) || !(height > 0)) return { cols: 1, tileWidth: 0 };
  let best = { cols: 1, tileWidth: 0 };
  for (let cols = 1; cols <= Math.max(1, count); cols++) {
    const rows = Math.ceil(Math.max(1, count) / cols);
    const cellW = (width - gap * (cols - 1)) / cols;
    const cellH = (height - gap * (rows - 1)) / rows;
    if (cellW <= 0 || cellH <= 0) continue;
    const tileWidth = Math.floor(Math.min(cellW, cellH * CALL_TILE_RATIO));
    if (tileWidth > best.tileWidth) best = { cols, tileWidth };
  }
  return best;
}

function callTilePages(tiles, perPage = PHONE_TILES_PER_PAGE) {
  const size = Math.max(1, Math.floor(perPage) || 1);
  const pages = [];
  for (let i = 0; i < (tiles || []).length; i += size) pages.push(tiles.slice(i, i + size));
  return pages;
}

function callGridLayout({ tiles, focusedStream }) {
  if (!tiles || !tiles.length) return "none";
  if (!focusedStream) return "grid";
  return tiles.some(t => !(t.kind === "screen" && t.user === focusedStream)) ? "strip" : "none";
}

function streamsToStop(watched, { tileUsers, focused }) {
  const keep = new Set(tileUsers || []);
  if (focused) keep.add(focused);
  return [...(watched || [])].filter(user => !keep.has(user));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { CALL_TILE_RATIO, PHONE_TILES_PER_PAGE, streamOpenAction, callTileKey, callTiles, callGridSize, callTilePages, callGridLayout, streamsToStop };
}
