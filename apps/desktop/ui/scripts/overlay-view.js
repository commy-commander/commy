const OVERLAY_CORNERS = ["top-left", "top-right", "bottom-left", "bottom-right"];
const OVERLAY_CHAT_MS = 8000;
const OVERLAY_CHAT_LINES = 4;
const OVERLAY_LINGER_MS = 1200;

const OVERLAY_NOTICE_MS = 3500;
const OVERLAY_TEST_MS = 6000;

const OVERLAY_CORNER_LABELS = { "top-left": "Top left", "top-right": "Top right", "bottom-left": "Bottom left", "bottom-right": "Bottom right" };

function overlayCornerLabel(corner) {
  return OVERLAY_CORNER_LABELS[overlayCorner(corner).name];
}

function overlayCorner(corner) {
  const name = OVERLAY_CORNERS.includes(corner) ? corner : "top-left";
  return { name, bottom: name.startsWith("bottom"), right: name.endsWith("right") };
}

function overlayVisibleMembers({ members, speaking, lingering, showEveryone }) {
  const list = members || [];
  if (showEveryone) return list;
  return list.filter(m => (speaking && speaking.has(m.id)) || (lingering && lingering.has(m.id)));
}

function overlayPushLine(lines, line, max = OVERLAY_CHAT_LINES) {
  return [...(lines || []), line].slice(-max);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { OVERLAY_NOTICE_MS, OVERLAY_TEST_MS, overlayCornerLabel, OVERLAY_CORNERS, OVERLAY_CHAT_MS, OVERLAY_CHAT_LINES, OVERLAY_LINGER_MS, overlayCorner, overlayVisibleMembers, overlayPushLine };
}
