const VOICE_ROOM_TILE = 16 / 9;
const VOICE_ROOM = {
  gap: 16,
  keyGap: 42,
  underGap: 26,
  seatGap: 12,
  tileMax: 480,
  cardRatio: 2.9,
  cardMin: 200,
  cardMax: 384,
  streamShare: 0.58,
  stripShare: 0.21,
  streamsAcross: 3,
  quarterPictures: 3,
  quarterCards: 4,
  compactFrom: 13,
  compactAcross: 5,
  oneRow: 6,
};
const VOICE_ROOM_TOOLS = [
  { id: "vr-invite", title: "Invite people", icon: "user-plus" },
  { id: "vr-activities", title: "Activities", icon: "gamepad-2" },
  { id: "vr-full", title: "Full screen", icon: "maximize-2" },
  { id: "vr-chat", title: "Hide chat", icon: "message-square" },
];

function voiceRoomCols(count) {
  if (count <= 1) return 1;
  if (count <= 4) return 2;
  if (count <= 9) return 3;
  if (count <= 12) return 4;
  return VOICE_ROOM.compactAcross;
}

function voiceRoomPlaceRows(rows, box, height, gap, top, rowGap = gap) {
  const placed = [];
  const block = rows.length * height + Math.max(0, rows.length - 1) * rowGap;
  const y0 = box.y + (top ? 0 : Math.max(0, (box.h - block) / 2));
  rows.forEach((row, r) => {
    const wide = row.reduce((sum, item) => sum + item.w, 0) + Math.max(0, row.length - 1) * gap;
    let x = box.x + (box.w - wide) / 2;
    for (const item of row) {
      placed.push({ key: item.key, form: item.form, x, y: y0 + r * (height + rowGap), w: item.w, h: height });
      x += item.w + gap;
    }
  });
  return { placed, bottom: y0 + block };
}

function voiceRoomChunks(items, size) {
  const rows = [];
  for (let i = 0; i < items.length; i += size) rows.push(items.slice(i, i + size));
  return rows;
}

function voiceRoomTiles(items, box, cols, limits = VOICE_ROOM) {
  const rows = Math.ceil(items.length / cols);
  const wide = Math.min((box.w - (cols - 1) * limits.gap) / cols, ((box.h - (rows - 1) * limits.gap) / rows) * VOICE_ROOM_TILE, limits.tileMax);
  const w = Math.max(0, wide);
  const sized = items.map(item => ({ key: item.key, form: item.kind === "person" ? "tile" : "picture", w }));
  return voiceRoomPlaceRows(voiceRoomChunks(sized, cols), box, w / VOICE_ROOM_TILE, limits.gap, false);
}

function voiceRoomCards(items, box, across, limits = VOICE_ROOM, exact = false) {
  const fits = exact ? across : Math.max(1, Math.floor((box.w + limits.gap) / (limits.cardMin + limits.gap)));
  const cols = Math.max(1, Math.min(across, fits, items.length));
  const rows = Math.ceil(items.length / cols);
  const byWidth = (box.w - (cols - 1) * limits.gap) / cols;
  const byHeight = ((box.h - (rows - 1) * limits.gap) / rows) * limits.cardRatio;
  const w = Math.max(0, Math.min(byWidth, limits.cardMax, Math.max(limits.cardMin, byHeight)));
  const h = w / limits.cardRatio;
  const sized = items.map(item => (item.kind === "camera"
    ? { key: item.key, form: "small", w: h * VOICE_ROOM_TILE }
    : { key: item.key, form: "compact", w }));
  return voiceRoomPlaceRows(voiceRoomChunks(sized, cols), box, h, limits.gap, false);
}

function voiceRoomQuarters(pictures, people, box, limits = VOICE_ROOM) {
  const cells = pictures.map(item => ({ key: item.key, form: "picture" }));
  if (people.length) cells.push({ key: "", form: "seats" });
  const cols = cells.length === 1 ? 1 : 2;
  const rows = Math.ceil(cells.length / cols);
  const w = Math.max(0, Math.min((box.w - (cols - 1) * limits.gap) / cols, ((box.h - (rows - 1) * limits.keyGap) / rows) * VOICE_ROOM_TILE));
  const h = w / VOICE_ROOM_TILE;
  const laid = voiceRoomPlaceRows(voiceRoomChunks(cells.map(cell => ({ ...cell, w })), cols), box, h, limits.gap, true, limits.keyGap);
  const placed = [];
  for (const cell of laid.placed) {
    if (cell.form !== "seats") {
      placed.push(cell);
      continue;
    }
    const sw = (cell.w - limits.seatGap) / 2;
    const sh = (cell.h - limits.seatGap) / 2;
    people.forEach((person, i) => {
      placed.push({ key: person.key, form: "seat", x: cell.x + (i % 2) * (sw + limits.seatGap), y: cell.y + Math.floor(i / 2) * (sh + limits.seatGap), w: sw, h: sh });
    });
  }
  return { placed, bottom: laid.bottom };
}

function voiceRoomExpanded(big, rest, box, limits = VOICE_ROOM) {
  const strip = rest.length ? Math.round(box.h * limits.stripShare) : 0;
  const high = Math.max(0, box.h - (strip ? strip + limits.underGap : 0));
  const w = Math.min(box.w, high * VOICE_ROOM_TILE);
  const h = w / VOICE_ROOM_TILE;
  const placed = [{ key: big.key, form: "big", x: box.x + (box.w - w) / 2, y: box.y, w, h }];
  if (!rest.length) return { placed, bottom: box.y + h };
  const mw = Math.max(0, Math.min(strip * VOICE_ROOM_TILE, (box.w - (rest.length - 1) * limits.gap) / rest.length));
  const mh = mw / VOICE_ROOM_TILE;
  const row = voiceRoomPlaceRows([rest.map(item => ({ key: item.key, form: "mini", w: mw }))], { x: box.x, y: box.y + h + limits.underGap, w: box.w, h: mh }, mh, limits.gap, true);
  return { placed: [...placed, ...row.placed], bottom: row.bottom };
}

function voiceRoomLayout({ width, height, items, expanded }, limits = VOICE_ROOM) {
  const list = items || [];
  const box = { x: 0, y: 0, w: Math.max(0, width || 0), h: Math.max(0, height || 0) };
  const big = expanded ? list.find(item => item.key === expanded) : null;
  if (big) return { mode: "expanded", ...voiceRoomExpanded(big, list.filter(item => item !== big), box, limits) };
  const streams = list.filter(item => item.kind === "stream");
  const cameras = list.filter(item => item.kind === "camera");
  const people = list.filter(item => item.kind === "person");
  if (!list.length || !box.w || !box.h) return { mode: "empty", placed: [], bottom: 0 };
  if (!streams.length) {
    const everyone = [...cameras, ...people];
    if (everyone.length < limits.compactFrom) return { mode: "people", ...voiceRoomTiles(everyone, box, voiceRoomCols(everyone.length), limits) };
    return { mode: "compact", ...voiceRoomCards(everyone, box, limits.compactAcross, limits, true) };
  }
  if (streams.length + cameras.length <= limits.quarterPictures && people.length <= limits.quarterCards) {
    return { mode: "quarters", ...voiceRoomQuarters([...streams, ...cameras], people, box, limits) };
  }
  const shown = streams.slice(0, limits.streamsAcross);
  const share = box.h * limits.streamShare;
  const w = Math.max(0, Math.min((box.w - (shown.length - 1) * limits.gap) / shown.length, share * VOICE_ROOM_TILE));
  const top = voiceRoomPlaceRows([shown.map(item => ({ key: item.key, form: "picture", w }))], { ...box, h: share }, w / VOICE_ROOM_TILE, limits.gap, false);
  const under = [...cameras, ...streams.slice(limits.streamsAcross).map(item => ({ key: item.key, kind: "waiting" })), ...people];
  if (!under.length) return { mode: "top", placed: top.placed, bottom: top.bottom };
  const rows = under.length <= limits.oneRow ? 1 : 2;
  const rest = { x: 0, y: share + limits.underGap, w: box.w, h: Math.max(0, box.h - share - limits.underGap) };
  const low = voiceRoomCards(under, rest, Math.ceil(under.length / rows), limits);
  return { mode: "top", placed: [...top.placed, ...low.placed], bottom: low.bottom };
}

function voiceRoomPeople({ userIds, names, muted, deafened, speaking, shares, cameras, meId, channelId }) {
  const talking = new Set(speaking || []);
  const onCamera = new Set(cameras || []);
  const live = new Set((shares || []).filter(share => !channelId || !share.channel || share.channel === channelId).map(share => share.user));
  return (userIds || []).map((id, i) => {
    const deaf = Boolean((deafened || [])[i]);
    const mute = Boolean((muted || [])[i]);
    const state = deaf ? "deafened" : mute ? "muted" : talking.has(id) ? "speaking" : "";
    return { id, name: (names || [])[i] || "", me: id === meId, live: live.has(id), camera: onCamera.has(id), state };
  });
}

function voiceRoomItems(tiles, people) {
  const streams = (tiles || []).filter(tile => tile.kind === "screen").map(tile => ({ key: tile.key, kind: "stream" }));
  const cameras = (tiles || []).filter(tile => tile.kind === "camera").map(tile => ({ key: tile.key, kind: "camera" }));
  const cards = (people || []).filter(person => !person.camera).map(person => ({ key: `person:${person.id}`, kind: "person" }));
  return [...streams, ...cameras, ...cards];
}

const VOICE_ROOM_QUALITY = {
  smooth: "best for games",
  balanced: "a good default",
  sharp: "best for text",
};

function voiceRoomQuality({ quality, lines, fps }) {
  const known = Object.prototype.hasOwnProperty.call(VOICE_ROOM_QUALITY, quality) ? quality : "balanced";
  const word = known[0].toUpperCase() + known.slice(1);
  const readout = [word, lines > 0 ? `${Math.round(lines)}p` : "", fps > 0 ? `${Math.round(fps)} fps` : ""].filter(Boolean).join(", ");
  return { word, rest: `: ${VOICE_ROOM_QUALITY[known]}`, label: `${word}: ${VOICE_ROOM_QUALITY[known]}`, readout };
}

function voiceRoomSub({ status, server }) {
  const text = String(status || "").trim();
  return text ? { text, kind: "status" } : { text: String(server || ""), kind: "server" };
}

function voiceRoomKey(doc, spec, makeIcon) {
  const key = doc.createElement("button");
  key.type = "button";
  key.id = spec.id;
  key.className = "vr-tool";
  key.title = spec.title;
  key.setAttribute("aria-label", spec.title);
  const icon = makeIcon ? makeIcon(spec.icon, 18) : null;
  if (icon) key.appendChild(icon);
  return key;
}

function mountStudioVoiceRoom(doc, makeIcon) {
  if (doc.documentElement.dataset.look !== "studio") return null;
  const present = doc.getElementById("voice-room");
  if (present) return present;
  const app = doc.getElementById("app");
  const main = app ? app.querySelector(".main") : null;
  if (!main) return null;
  const room = doc.createElement("section");
  room.id = "voice-room";
  room.className = "voice-room hidden";
  room.setAttribute("aria-label", "Voice room");
  const head = doc.createElement("header");
  head.className = "vr-head";
  const fold = voiceRoomKey(doc, { id: "vr-fold", title: "Show channels", icon: "panel-left" }, makeIcon);
  fold.className = "vr-tool vr-fold";
  fold.setAttribute("aria-pressed", "false");
  const title = doc.createElement("h2");
  title.className = "vr-title";
  const mark = doc.createElement("span");
  mark.className = "vr-spk";
  const speaker = makeIcon ? makeIcon("volume-2", 21) : null;
  if (speaker) mark.appendChild(speaker);
  const name = doc.createElement("span");
  name.id = "vr-name";
  name.className = "vr-name";
  title.append(mark, name);
  const sub = doc.createElement("button");
  sub.type = "button";
  sub.id = "vr-sub";
  sub.className = "vr-sub";
  const clock = doc.createElement("span");
  clock.id = "vr-clock";
  clock.className = "vr-clock";
  clock.title = "Time in the room";
  const tools = doc.createElement("div");
  tools.className = "vr-tools";
  for (const spec of VOICE_ROOM_TOOLS) tools.appendChild(voiceRoomKey(doc, spec, makeIcon));
  head.append(fold, title, sub, clock, tools);
  const grid = doc.createElement("div");
  grid.id = "vr-grid";
  grid.className = "vr-grid";
  const cards = doc.createElement("div");
  cards.id = "vr-cards";
  cards.className = "vr-cards";
  cards.setAttribute("aria-label", "In the room");
  grid.appendChild(cards);
  room.append(head, grid);
  main.after(room);
  const chatHead = main.querySelector(".chat-header");
  if (chatHead) {
    const shut = voiceRoomKey(doc, { id: "vr-chat-x", title: "Hide chat", icon: "x" }, makeIcon);
    shut.className = "icon-btn vr-wing-x hidden";
    chatHead.appendChild(shut);
  }
  return room;
}

if (typeof document !== "undefined") {
  mountStudioVoiceRoom(document, typeof studioIconNode === "undefined" ? null : (name, size) => studioIconNode(name, size, false));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { VOICE_ROOM, VOICE_ROOM_TILE, VOICE_ROOM_TOOLS, VOICE_ROOM_QUALITY, voiceRoomCols, voiceRoomPlaceRows, voiceRoomTiles, voiceRoomCards, voiceRoomQuarters, voiceRoomExpanded, voiceRoomLayout, voiceRoomQuality, voiceRoomPeople, voiceRoomItems, voiceRoomSub, mountStudioVoiceRoom };
}
