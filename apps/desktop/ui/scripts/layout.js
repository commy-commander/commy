const NARROW_WINDOW_PX = 1200;

function membersCollapsed(pref, width) {
  if (pref === "1") return true;
  if (pref === "0") return false;
  return width < NARROW_WINDOW_PX;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { NARROW_WINDOW_PX, membersCollapsed };
}
