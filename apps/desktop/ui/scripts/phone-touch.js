const PHONE_TABS = ["home", "messages", "live", "you"];
const HORIZON = { baseline: 30, swell: 9, spread: 25, pingSpread: 700, step: 3, overhang: 6 };
const HORIZON_SPRINGS = { slide: { k: 230, c: 21 }, hop: { k: 420, c: 23 }, knock: { k: 260, c: 17 } };
const HANG_UP_FUSE_MS = 3000;
const STACK_MAX = 4;

function horizonY(x, centre, swell, pingCentre, ping) {
  const main = swell * Math.exp(-((x - centre) ** 2) / (2 * HORIZON.spread ** 2));
  const knock = ping && pingCentre !== null ? ping * Math.exp(-((x - pingCentre) ** 2) / HORIZON.pingSpread) : 0;
  return HORIZON.baseline - main - knock;
}

function horizonPath(width, centre, swell = HORIZON.swell, pingCentre = null, ping = 0) {
  const parts = [];
  for (let x = -HORIZON.overhang; x <= width + HORIZON.overhang; x += HORIZON.step) {
    parts.push(`${parts.length ? "L" : "M"}${x} ${horizonY(x, centre, swell, pingCentre, ping).toFixed(2)}`);
  }
  return parts.join("");
}

function horizonLift(tabCentre, centre, swell = HORIZON.swell) {
  return -swell * Math.exp(-((tabCentre - centre) ** 2) / (2 * HORIZON.spread ** 2));
}

function springStep(spring, seconds) {
  const dt = Math.min(0.034, Math.max(0.001, seconds));
  const slices = Math.ceil(dt / 0.004);
  const h = dt / slices;
  let { x, v } = spring;
  for (let i = 0; i < slices; i++) {
    v += (-spring.k * (x - spring.target) - spring.c * v) * h;
    x += v * h;
  }
  const settled = Math.abs(x - spring.target) < 0.03 && Math.abs(v) < 0.6;
  return settled ? { ...spring, x: spring.target, v: 0, settled } : { ...spring, x, v, settled };
}

function tabForConversation(view) {
  return view === "dm" ? "messages" : "home";
}

function phoneTabAfter(tab, { pane, view }) {
  const known = PHONE_TABS.includes(tab) ? tab : "home";
  if (pane !== "list") return tabForConversation(view);
  if (known === "live" || known === "you") return known;
  return tabForConversation(view);
}

function hangUpTap(armed) {
  return armed ? "leave" : "arm";
}

function badgeText(count) {
  const n = Math.floor(Number(count) || 0);
  if (n <= 0) return "";
  return n > 99 ? "99+" : String(n);
}

function roomLine(count, connected) {
  if (connected) return "You're connected";
  if (count <= 0) return "";
  return count === 1 ? "1 person in the room" : `${count} in the room`;
}

function liveSummary(streams, rooms) {
  if (streams > 0) return streams === 1 ? "1 stream, right now" : `${streams} streams, right now`;
  if (rooms > 0) return rooms === 1 ? "1 room has people in it" : `${rooms} rooms have people in them`;
  return "";
}

function roomPeople(channel) {
  const names = channel.voice_users || [];
  const ids = channel.voice_user_ids || [];
  return names.map((name, i) => ({ id: ids[i] || null, name }));
}

function liveRooms(servers, me = {}) {
  const rooms = [];
  for (const server of servers || []) {
    if (!server || server.group) continue;
    const shares = server.screen_shares || [];
    for (const channel of server.channels || []) {
      if (channel.kind !== "voice" && channel.kind !== "stage") continue;
      const people = roomPeople(channel);
      if (!people.length) continue;
      const streams = shares
        .filter(share => (share.channel ? share.channel === channel.id : people.some(person => person.id === share.user)))
        .map(share => ({ userId: share.user, name: share.name, mine: share.user === me.userId }));
      rooms.push({
        serverId: server.id,
        serverName: server.name,
        channelId: channel.id,
        channelName: channel.name,
        people,
        streams,
        here: Boolean(me.channelId && me.channelId === channel.id),
      });
    }
  }
  return rooms;
}

function liveStreams(rooms) {
  return rooms.flatMap(room => room.streams.map(stream => ({ ...stream, room })));
}

function roomFaces(people, me) {
  return me ? people.filter(person => person.id !== me) : people;
}

function stackPeople(people, max = STACK_MAX) {
  const shown = people.slice(0, max);
  return { shown, more: Math.max(0, people.length - shown.length) };
}

const TILE_NAME = { cells: 1.5, margin: 8, gap: 2 };

function tileNameWidth(cell, edgeRoom, sideRooms, limits = TILE_NAME) {
  const spare = Math.min((limits.cells - 1) * cell / 2, edgeRoom, ...sideRooms.map(room => room - limits.gap));
  return cell + 2 * Math.max(0, spare);
}

const WAIT_CARDS = 3;

function waitCards(count, open, max = WAIT_CARDS) {
  const shown = open ? count : Math.min(count, max);
  return { shown, more: count - shown };
}

function waitMoreWords(more) {
  return `${more} more request${more === 1 ? "" : "s"}`;
}

function listAge(iso, now, locale) {
  const then = iso ? new Date(iso).getTime() : NaN;
  if (Number.isNaN(then)) return "";
  const minutes = Math.floor((now - then) / 60000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)}h`;
  if (minutes < 10080) return new Date(then).toLocaleDateString(locale || [], { weekday: "short" });
  return new Date(then).toLocaleDateString(locale || [], { month: "short", day: "numeric" });
}

function unreadChats(counts) {
  return counts.filter(count => count > 0).length;
}

function listLabel(typed) {
  if (/^Friends - \d+$/.test(typed)) return "Friends";
  return typed.replace(/ ([A-Z])(?=[a-z])/g, (_, letter) => " " + letter.toLowerCase());
}

const SWIPE = { slop: 6, replyAt: 56, replyMax: 84, edge: 24, backAt: 80, rowOpen: 152, rowAt: 60, sheetAt: 72, chipAfterMs: 110 };
const QUICK_REACTIONS = ["\u{1F44D}", "\u2764\uFE0F", "\u{1F602}", "\u{1F389}", "\u{1F525}"];
const SHEET_ROWS = {
  "Reply": { label: "Reply", icon: "reply", hint: "or swipe right" },
  "Open Thread": { label: "Open thread", icon: "message-square-text" },
  "Pin": { label: "Pin to channel", icon: "pin" },
  "Unpin": { label: "Unpin", icon: "pin" },
  "Edit": { label: "Edit", icon: "pencil" },
  "Hide for Me": { label: "Hide for me", icon: "eye-off" },
  "Save": { label: "Save message", icon: "bookmark" },
  "Unsave": { label: "Remove from saved", icon: "bookmark" },
  "Remind Me": { label: "Remind me", icon: "bell" },
  "Copy text": { label: "Copy text", icon: "copy" },
  "More": { label: "More", icon: "ellipsis" },
  "Report Message": { label: "Report message", icon: "flag" },
  "Delete": { label: "Delete", icon: "trash-2", danger: true },
  "Delete (Moderator)": { label: "Delete as moderator", icon: "trash-2", danger: true },
};

function swipeAxis(dx, dy) {
  if (Math.abs(dx) < SWIPE.slop && Math.abs(dy) < SWIPE.slop) return null;
  return Math.abs(dx) > Math.abs(dy) * 1.5 ? "x" : "y";
}

function replyPull(dx) {
  if (!(dx > 0)) return 0;
  return Math.min(SWIPE.replyMax, dx);
}

function replyCommits(dx) {
  return dx >= SWIPE.replyAt;
}

function edgeBackStarts(x) {
  return x <= SWIPE.edge;
}

function edgeBackCommits(dx) {
  return dx >= SWIPE.backAt;
}

function rowPull(dx, wasOpen) {
  const raw = (wasOpen ? SWIPE.rowOpen : 0) + dx;
  if (raw <= 0) return 0;
  if (raw <= SWIPE.rowOpen) return raw;
  return Math.min(SWIPE.rowOpen + 18, SWIPE.rowOpen + (raw - SWIPE.rowOpen) * 0.25);
}

function rowStaysOpen(pull, wasOpen) {
  return wasOpen ? pull > SWIPE.rowOpen - SWIPE.rowAt : pull >= SWIPE.rowAt;
}

function sheetPull(dy) {
  return Math.max(0, Number(dy) || 0);
}

function sheetCloses(dy) {
  return sheetPull(dy) >= SWIPE.sheetAt;
}

const BAR_EXTRAS = [
  { id: "emoji", label: "Emoji", icon: "smile", press: "emoji-btn" },
  { id: "style", label: "Text Style", icon: "a-large-small", press: "text-style-btn" },
];

function sheetRow(title) {
  if (SHEET_ROWS[title]) return SHEET_ROWS[title];
  const words = String(title || "").trim();
  return { label: words ? words[0].toUpperCase() + words.slice(1).toLowerCase() : "", icon: null };
}

function quickReactions(frequent, limit = QUICK_REACTIONS.length) {
  const picked = [];
  for (const emoji of [...(frequent || []), ...QUICK_REACTIONS]) {
    if (typeof emoji === "string" && emoji && !emoji.startsWith(":") && !picked.includes(emoji)) picked.push(emoji);
    if (picked.length === limit) break;
  }
  return picked;
}

function chatSubtitle({ view, serverName, group, presence }) {
  if (view === "dm") return presence || "Direct message";
  if (group) return "Group chat";
  return serverName || "";
}

const ROOM_KEYS = [
  { id: "sounds", label: "Sounds", icon: "music" },
  { id: "camera", label: "Camera", icon: "video-off" },
  { id: "mic", label: "Mic", icon: "mic" },
  { id: "deafen", label: "Deafen", icon: "headphones" },
  { id: "leave", label: "Leave", icon: "phone-off" },
];
const SIGNAL_WORDS = { good: "Good", ok: "Fair", bad: "Poor" };

function roomTiles(channel, me = {}) {
  if (!channel) return [];
  const muted = channel.voice_muted || [];
  const deafened = channel.voice_deafened || [];
  return roomPeople(channel).map((person, i) => {
    const you = Boolean(me.userId && person.id === me.userId);
    return {
      id: person.id,
      name: person.name,
      label: you ? "You" : person.name,
      you,
      muted: you ? Boolean(me.muted) : Boolean(muted[i]),
      deafened: you ? Boolean(me.deafened) : Boolean(deafened[i]),
    };
  });
}

function duoTiles(name, me = {}) {
  return [
    { id: me.otherId || null, name: name || "Call", label: name || "Call", you: false, muted: false, deafened: false },
    { id: me.userId || null, name: me.name || "You", label: "You", you: true, muted: Boolean(me.muted), deafened: Boolean(me.deafened) },
  ];
}

function roomHeading({ serverName, count, call }) {
  if (call) return "Voice call";
  const here = count === 1 ? "1 here" : `${count} here`;
  return serverName ? `${serverName} \u00B7 ${here}` : here;
}

function roomStreams(server, channelId, me = {}) {
  if (!server || !channelId) return [];
  const room = liveRooms([server], me).find(found => found.channelId === channelId);
  if (!room) return [];
  return room.streams
    .filter(stream => !stream.mine && stream.userId !== me.userId)
    .map(stream => ({ user: stream.userId, name: stream.name, title: `${stream.name}'s screen`, watching: stream.userId === me.watching }));
}

function signalWord(quality) {
  return SIGNAL_WORDS[quality] || "";
}

function roomKeyState(id, voice = {}) {
  if (id === "mic") return { down: Boolean(voice.muted), icon: voice.muted ? "mic-off" : "mic", label: voice.muted ? "Unmute" : "Mute" };
  if (id === "deafen") return { down: Boolean(voice.deafened), icon: "headphones", label: voice.deafened ? "Undeafen" : "Deafen" };
  if (id === "camera") return { down: false, off: !voice.camera, icon: voice.camera ? "video" : "video-off", label: voice.camera ? "Turn off camera" : "Turn on camera" };
  if (id === "sounds") return { down: Boolean(voice.sounds), icon: "music", label: "Soundboard" };
  return { down: false, icon: "phone", label: "Hang up" };
}

function roomSignature(tiles, streams, extra = "") {
  return [
    tiles.map(tile => `${tile.id}:${tile.name}:${tile.muted ? 1 : 0}${tile.deafened ? 1 : 0}`).join(","),
    streams.map(stream => `${stream.user}:${stream.watching ? 1 : 0}`).join(","),
    extra,
  ].join("|");
}

const BUBBLE_MAX = 3;
const BUBBLE_CHARS = 72;

function streamBubbles(rows, limit = BUBBLE_MAX) {
  const said = [];
  for (const row of rows || []) {
    const text = String((row && row.text) || "").replace(/\s+/g, " ").trim();
    if (!text || !row.author) continue;
    said.push({ id: row.id || null, author: row.author, color: row.color || "", face: row.face || null, text: text.length > BUBBLE_CHARS ? `${text.slice(0, BUBBLE_CHARS - 1).trimEnd()}\u2026` : text });
  }
  return said.slice(-limit);
}

function watchPlace(voiceServer) {
  if (!voiceServer) return "";
  if (voiceServer.call) return `Call with ${voiceServer.name}`;
  return [voiceServer.channelName, voiceServer.name].filter(Boolean).join(" \u00B7 ");
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    PHONE_TABS,
    HORIZON,
    HORIZON_SPRINGS,
    HANG_UP_FUSE_MS,
    STACK_MAX,
    TILE_NAME,
    tileNameWidth,
    WAIT_CARDS,
    waitCards,
    waitMoreWords,
    listAge,
    listLabel,
    unreadChats,
    roomFaces,
    horizonY,
    horizonPath,
    horizonLift,
    springStep,
    tabForConversation,
    phoneTabAfter,
    hangUpTap,
    badgeText,
    roomLine,
    liveSummary,
    roomPeople,
    liveRooms,
    liveStreams,
    stackPeople,
    SWIPE,
    QUICK_REACTIONS,
    SHEET_ROWS,
    swipeAxis,
    replyPull,
    replyCommits,
    edgeBackStarts,
    edgeBackCommits,
    rowPull,
    rowStaysOpen,
    sheetPull,
    sheetCloses,
    BAR_EXTRAS,
    sheetRow,
    quickReactions,
    chatSubtitle,
    ROOM_KEYS,
    roomTiles,
    duoTiles,
    roomHeading,
    roomStreams,
    signalWord,
    roomKeyState,
    roomSignature,
    streamBubbles,
    watchPlace,
  };
}
