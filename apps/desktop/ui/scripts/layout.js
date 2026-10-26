const NARROW_WINDOW_PX = 1200;
const CHAT_FIRST_PX = 960;

const SIDEBAR_MIN_PX = 180;
const SIDEBAR_MAX_PX = 420;

function clampSidebarWidth(width, windowWidth) {
  const cap = Math.max(SIDEBAR_MIN_PX, Math.min(SIDEBAR_MAX_PX, Math.floor(windowWidth * 0.45)));
  return Math.round(Math.min(cap, Math.max(SIDEBAR_MIN_PX, width)));
}

function membersCollapsed(pref, width) {
  if (width < CHAT_FIRST_PX) return true;
  if (pref === "1") return true;
  if (pref === "0") return false;
  return width < NARROW_WINDOW_PX;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { NARROW_WINDOW_PX, CHAT_FIRST_PX, SIDEBAR_MIN_PX, SIDEBAR_MAX_PX, clampSidebarWidth, membersCollapsed };
}
