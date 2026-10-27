const NARROW_WINDOW_PX = 1200;
const CHAT_FIRST_PX = 960;

const SIDEBAR_MIN_PX = 180;
const SIDEBAR_MAX_PX = 420;

function clampSidebarWidth(width, windowWidth) {
  const cap = Math.max(SIDEBAR_MIN_PX, Math.min(SIDEBAR_MAX_PX, Math.floor(windowWidth * 0.45)));
  return Math.round(Math.min(cap, Math.max(SIDEBAR_MIN_PX, width)));
}

function pickConnLink(links, picked) {
  return links.find(l => l.user === picked) || links[0] || null;
}

function membersCollapsed(pref, width) {
  if (width < CHAT_FIRST_PX) return true;
  if (pref === "1") return true;
  if (pref === "0") return false;
  return width < NARROW_WINDOW_PX;
}

const NARROW_LAYOUTS = ["stack", "slim", "switcher"];
const VOICE_RING_LIMITS = { stack: 6, slim: 4, switcher: 4 };

function narrowLayoutFromQuery(search) {
  const asked = new URLSearchParams(search).get("narrow");
  return NARROW_LAYOUTS.includes(asked) ? asked : null;
}

function voiceRingSplit(count, layout) {
  const limit = VOICE_RING_LIMITS[layout] || count;
  if (count <= limit) return { shown: count, more: 0 };
  return { shown: limit - 1, more: count - limit + 1 };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { NARROW_WINDOW_PX, CHAT_FIRST_PX, SIDEBAR_MIN_PX, SIDEBAR_MAX_PX, clampSidebarWidth, pickConnLink, membersCollapsed, NARROW_LAYOUTS, narrowLayoutFromQuery, voiceRingSplit };
}
