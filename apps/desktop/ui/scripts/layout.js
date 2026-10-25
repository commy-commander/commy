const NARROW_WINDOW_PX = 1200;
const CHAT_FIRST_PX = 960;

function membersCollapsed(pref, width) {
  if (width < CHAT_FIRST_PX) return true;
  if (pref === "1") return true;
  if (pref === "0") return false;
  return width < NARROW_WINDOW_PX;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { NARROW_WINDOW_PX, CHAT_FIRST_PX, membersCollapsed };
}
