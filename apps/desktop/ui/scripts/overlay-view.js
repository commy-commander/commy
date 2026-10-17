// What the in-game overlay shows, kept free of DOM and backend so it
// runs in the overlay window and in `node --test` alike (see
// ui/tests/overlay-view.test.mjs).

const OVERLAY_CORNERS = ["top-left", "top-right", "bottom-left", "bottom-right"];
// A chat line stays this long, and at most this many show at once.
const OVERLAY_CHAT_MS = 8000;
const OVERLAY_CHAT_LINES = 4;
// Someone who stops talking stays lit a moment so the list does not flicker.
const OVERLAY_LINGER_MS = 1200;

// How long the "overlay is here" note and the Test overlay sample stay.
const OVERLAY_NOTICE_MS = 3500;
const OVERLAY_TEST_MS = 6000;

const OVERLAY_CORNER_LABELS = { "top-left": "Top left", "top-right": "Top right", "bottom-left": "Bottom left", "bottom-right": "Bottom right" };

// "Top left" for the note the overlay shows when it appears or moves.
function overlayCornerLabel(corner) {
  return OVERLAY_CORNER_LABELS[overlayCorner(corner).name];
}

// Which way the overlay hugs the screen: bottom corners sit on the
// bottom edge, right corners on the right edge. Unknown names mean top-left.
function overlayCorner(corner) {
  const name = OVERLAY_CORNERS.includes(corner) ? corner : "top-left";
  return { name, bottom: name.startsWith("bottom"), right: name.endsWith("right") };
}

// Who is drawn: only people talking (or just stopped), or everyone in
// the channel when the viewer asked for that. `speaking` and `lingering`
// are sets of ids.
function overlayVisibleMembers({ members, speaking, lingering, showEveryone }) {
  const list = members || [];
  if (showEveryone) return list;
  return list.filter(m => (speaking && speaking.has(m.id)) || (lingering && lingering.has(m.id)));
}

// New chat line appended, oldest dropped past the limit.
function overlayPushLine(lines, line, max = OVERLAY_CHAT_LINES) {
  return [...(lines || []), line].slice(-max);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { OVERLAY_NOTICE_MS, OVERLAY_TEST_MS, overlayCornerLabel, OVERLAY_CORNERS, OVERLAY_CHAT_MS, OVERLAY_CHAT_LINES, OVERLAY_LINGER_MS, overlayCorner, overlayVisibleMembers, overlayPushLine };
}
