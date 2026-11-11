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

function overlayTileKey(member, picturesOnly, studio = false) {
  const key = [member.name || "", member.image || "", member.emoji || "", Boolean(picturesOnly)];
  if (studio) key.push("studio", member.face || "");
  return JSON.stringify(key);
}

function overlayPushLine(lines, line, max = OVERLAY_CHAT_LINES) {
  return [...(lines || []), line].slice(-max);
}

const OVERLAY_STAR = [
  ["#1b2fb0", "M1.04 37.56A3.2 3.2 0 0 1 1.19 36.57L4.07 27.72A3.2 3.2 0 0 1 6.6 25.55L27.5 22.14A3 3 0 0 0 29.68 20.56L39.38 1.73A3.2 3.2 0 0 1 42.23 0L51.52 0A3.2 3.2 0 0 1 54.37 1.73L64.07 20.56A3 3 0 0 0 66.25 22.14L87.15 25.55A3.2 3.2 0 0 1 89.68 27.72L92.56 36.57A3.2 3.2 0 0 1 92.71 37.56v12.5A3.2 3.2 0 0 1 91.79 52.31L77.48 66.75L79.26 78.35A3.2 3.2 0 0 1 79.3 78.83v12.5A3.2 3.2 0 0 1 77.98 93.92L70.46 99.39A3.2 3.2 0 0 1 67.14 99.66L48.22 90.13A3 3 0 0 0 45.53 90.13L26.61 99.66A3.2 3.2 0 0 1 23.29 99.39L15.77 93.92A3.2 3.2 0 0 1 14.45 91.33v-12.5A3.2 3.2 0 0 1 14.49 78.35L16.27 66.75L1.96 52.31A3.2 3.2 0 0 1 1.04 50.06Z"],
  ["#3355ff", "M39.38 1.73A3.2 3.2 0 0 1 42.23 0L51.52 0A3.2 3.2 0 0 1 54.37 1.73L64.07 20.56A3 3 0 0 0 66.25 22.14L87.15 25.55A3.2 3.2 0 0 1 89.68 27.72L92.56 36.57A3.2 3.2 0 0 1 91.79 39.81L76.88 54.85A3 3 0 0 0 76.05 57.42L79.26 78.35A3.2 3.2 0 0 1 77.98 81.42L70.46 86.89A3.2 3.2 0 0 1 67.14 87.16L48.22 77.63A3 3 0 0 0 45.53 77.63L26.61 87.16A3.2 3.2 0 0 1 23.29 86.89L15.77 81.42A3.2 3.2 0 0 1 14.49 78.35L17.7 57.42A3 3 0 0 0 16.87 54.85L1.96 39.81A3.2 3.2 0 0 1 1.19 36.57L4.07 27.72A3.2 3.2 0 0 1 6.6 25.55L27.5 22.14A3 3 0 0 0 29.68 20.56Z"],
  ["#eef0f4", "M33 43.75h2.75a1.75 1.75 0 0 1 1.75 1.75v2.75a1.75 1.75 0 0 1 -1.75 1.75h-2.75a1.75 1.75 0 0 1 -1.75 -1.75v-2.75a1.75 1.75 0 0 1 1.75 -1.75ZM45.5 43.75h2.75a1.75 1.75 0 0 1 1.75 1.75v2.75a1.75 1.75 0 0 1 -1.75 1.75h-2.75a1.75 1.75 0 0 1 -1.75 -1.75v-2.75a1.75 1.75 0 0 1 1.75 -1.75ZM58 43.75h2.75a1.75 1.75 0 0 1 1.75 1.75v2.75a1.75 1.75 0 0 1 -1.75 1.75h-2.75a1.75 1.75 0 0 1 -1.75 -1.75v-2.75a1.75 1.75 0 0 1 1.75 -1.75Z"],
];

function overlayPicture(member, studio) {
  if (member && member.image) return { kind: "image", value: member.image };
  if (member && member.emoji) return { kind: "emoji", value: member.emoji };
  if (studio) return { kind: "face", value: String((member && (member.face || member.id)) || "") };
  return { kind: "initials", value: String((member && member.name) || "?").split(/\s+/).filter(Boolean).slice(0, 2).map(w => w[0].toUpperCase()).join("") || "?" };
}

function overlayTestMember(me, studio) {
  const base = { id: "overlay-test", name: "You, talking" };
  if (!studio || !me) return base;
  return { ...base, face: me.id || "", image: me.image || null, emoji: me.emoji || null };
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

function overlayPlaceKey(config) {
  const layout = config && config.layout;
  if (!layout) return overlayCorner(config && config.corner).name;
  return ["voice", "chat"].map(k => `${(layout[k] || {}).x},${(layout[k] || {}).y}`).join("|");
}

function overlayMovedNotice(previous, config) {
  if (!previous || overlayPlaceKey(previous) === overlayPlaceKey(config)) return null;
  return config.layout ? "Overlay on · Your layout" : `Overlay on · ${overlayCornerLabel(config.corner)}`;
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
  module.exports = { OVERLAY_STAR, overlayPicture, overlayTestMember, OVERLAY_MAX_LAYOUTS, OVERLAY_MAX_LAYOUT_NAME, OVERLAY_SNAP, OVERLAY_CHAT_LABEL, overlaySnap, overlayPart, overlayPlaceKey, overlayMovedNotice, overlayStartLayout, overlaySameLayout, overlayLayoutName, overlaySaveLayout, OVERLAY_NOTICE_MS, OVERLAY_TEST_MS, overlayCornerLabel, OVERLAY_CORNERS, OVERLAY_CHAT_MS, OVERLAY_CHAT_LINES, OVERLAY_LINGER_MS, overlayCorner, overlayUniqueMembers, overlayVisibleMembers, overlayTileKey, overlayPushLine };
}
