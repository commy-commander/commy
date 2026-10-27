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

function overlayUniqueMembers(members) {
  const seen = new Set();
  return (members || []).filter(m => {
    if (seen.has(m.id)) return false;
    seen.add(m.id);
    return true;
  });
}

function overlayVisibleMembers({ members, speaking, lingering, showEveryone, picturesOnly }) {
  const list = overlayUniqueMembers(members);
  if (showEveryone || picturesOnly) return list;
  return list.filter(m => (speaking && speaking.has(m.id)) || (lingering && lingering.has(m.id)));
}

function overlayPushLine(lines, line, max = OVERLAY_CHAT_LINES) {
  return [...(lines || []), line].slice(-max);
}

const OVERLAY_MAX_LAYOUTS = 8;
const OVERLAY_MAX_LAYOUT_NAME = 32;
const OVERLAY_SNAP = 0.04;
const OVERLAY_CHAT_LABEL = "overlay-chat";

function overlayClamp(value) {
  const n = Number(value);
  return Number.isFinite(n) ? Math.min(1, Math.max(0, n)) : 0;
}

function overlaySnap(value) {
  const n = overlayClamp(value);
  if (n < OVERLAY_SNAP) return 0;
  if (n > 1 - OVERLAY_SNAP) return 1;
  if (Math.abs(n - 0.5) < OVERLAY_SNAP / 2) return 0.5;
  return Math.round(n * 1000) / 1000;
}

function overlayPart(config, label) {
  const layout = config && config.layout;
  if (!layout) {
    const corner = overlayCorner(config && config.corner);
    return { part: "both", bottom: corner.bottom, right: corner.right };
  }
  const part = label === OVERLAY_CHAT_LABEL ? "chat" : "voice";
  const spot = layout[part] || { x: 0, y: 0 };
  return { part, bottom: spot.y > 0.5, right: spot.x > 0.5 };
}

function overlayStartLayout(corner) {
  const c = overlayCorner(corner);
  const x = c.right ? 1 : 0;
  return { voice: { x, y: c.bottom ? 1 : 0 }, chat: { x, y: c.bottom ? 0 : 1 } };
}

function overlaySameLayout(a, b) {
  if (!a || !b) return false;
  return ["voice", "chat"].every(k => a[k].x === b[k].x && a[k].y === b[k].y);
}

function overlayLayoutName(name) {
  return [...String(name || "").trim()].filter(c => !/[\u0000-\u001f\u007f-\u009f]/.test(c)).slice(0, OVERLAY_MAX_LAYOUT_NAME).join("");
}

function overlaySaveLayout(layouts, name, layout) {
  const clean = overlayLayoutName(name);
  if (!clean || !layout) return layouts || [];
  const kept = (layouts || []).filter(l => l.name.toLowerCase() !== clean.toLowerCase());
  return [...kept, { name: clean, voice: { ...layout.voice }, chat: { ...layout.chat } }].slice(-OVERLAY_MAX_LAYOUTS);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { OVERLAY_MAX_LAYOUTS, OVERLAY_MAX_LAYOUT_NAME, OVERLAY_SNAP, OVERLAY_CHAT_LABEL, overlaySnap, overlayPart, overlayStartLayout, overlaySameLayout, overlayLayoutName, overlaySaveLayout, OVERLAY_NOTICE_MS, OVERLAY_TEST_MS, overlayCornerLabel, OVERLAY_CORNERS, OVERLAY_CHAT_MS, OVERLAY_CHAT_LINES, OVERLAY_LINGER_MS, overlayCorner, overlayUniqueMembers, overlayVisibleMembers, overlayPushLine };
}
