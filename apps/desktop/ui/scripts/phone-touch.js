const PHONE_TABS = ["home", "messages", "live", "you"];
const HORIZON = { baseline: 30, swell: 9, spread: 25, roomSpread: 35, hot: 62, roomHot: 70, pingSpread: 700, step: 3, overhang: 6 };
const HORIZON_SPRINGS = { slide: { k: 230, c: 21 }, hop: { k: 420, c: 23 }, knock: { k: 260, c: 17 } };
const STACK_MAX = 4;

function horizonY(x, centre, swell, pingCentre, ping, spread = HORIZON.spread) {
  const main = swell * Math.exp(-((x - centre) ** 2) / (2 * spread ** 2));
  const knock = ping && pingCentre !== null ? ping * Math.exp(-((x - pingCentre) ** 2) / HORIZON.pingSpread) : 0;
  return HORIZON.baseline - main - knock;
}

function horizonPath(width, centre, swell = HORIZON.swell, pingCentre = null, ping = 0, spread = HORIZON.spread) {
  const parts = [];
  for (let x = -HORIZON.overhang; x <= width + HORIZON.overhang; x += HORIZON.step) {
    parts.push(`${parts.length ? "L" : "M"}${x} ${horizonY(x, centre, swell, pingCentre, ping, spread).toFixed(2)}`);
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

function badgeText(count) {
  const n = Math.floor(Number(count) || 0);
  if (n <= 0) return "";
  return n > 99 ? "99+" : String(n);
}

function roomLine(count, connected) {
  if (connected) return t("ui.you-re-connected");
  if (count <= 0) return "";
  return count === 1 ? t("ui.room.people.one") : t("ui.room.people.many", { count });
}

function liveSummary(streams, rooms) {
  if (streams > 0) return streams === 1 ? t("ui.live.streams.one") : t("ui.live.streams.many", { count: streams });
  if (rooms > 0) return rooms === 1 ? t("ui.live.rooms.one") : t("ui.live.rooms.many", { count: rooms });
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

const WATCH_STEPS = [
  { faces: 4, server: true },
  { faces: 3, server: true },
  { faces: 1, server: true },
  { faces: 1, server: false },
];

function watchStack(people, step, sharer) {
  if (step.faces > 1) return { one: false, ...stackPeople(people, step.faces) };
  const lead = people.find(person => person.id === sharer) || people[0];
  return { one: true, shown: people.map(person => ({ ...person, lead: person === lead })), more: Math.max(0, people.length - 1) };
}

const STREAM_TOOLS_MS = 4000;

function streamToolsStay(tool) {
  return Boolean(tool);
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
  return more === 1 ? t("ui.requests.more.one") : t("ui.requests.more.many", { count: more });
}

function listAge(iso, now, locale) {
  const then = iso ? new Date(iso).getTime() : NaN;
  if (Number.isNaN(then)) return "";
  const minutes = Math.floor((now - then) / 60000);
  if (minutes < 1) return t("ui.age.now");
  if (minutes < 60) return `${minutes}m`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)}h`;
  if (minutes < 10080) return new Date(then).toLocaleDateString(locale || dateLocale(), { weekday: "short" });
  return new Date(then).toLocaleDateString(locale || dateLocale(), { month: "short", day: "numeric" });
}

function unreadChats(counts) {
  return counts.filter(count => count > 0).length;
}

function listLabel(typed) {
  const friends = t("ui.friends-2");
  if (/^Friends - \d+$/.test(typed) || new RegExp(`^${friends} - \\d+$`).test(typed)) return friends;
  return typed.replace(/ ([A-Z])(?=[a-z])/g, (_, letter) => " " + letter.toLowerCase());
}

const SWIPE = { slop: 10, lean: 1.2, edgeLean: 0.6, scrollSlop: 4, replyAt: 56, replyMax: 84, edge: 30, rowOpen: 112, rowAt: 60, rowFlick: 0.45, rowFlickAt: 24, sheetAt: 72, chipAfterMs: 110 };
const TRAY_PULL = 40;
const QUICK_REACTIONS = ["\u{1F44D}", "\u2764\uFE0F", "\u{1F602}", "\u{1F389}", "\u{1F525}"];
const SHEET_ROWS = {
  "Reply": { get label() { return t("ui.reply"); }, icon: "reply", get hint() { return t("ui.or-swipe-the-message-right"); } },
  "Copy Image": { get label() { return t("ui.copy-image"); }, icon: "copy" },
  "Save Image": { get label() { return t("ui.save-image"); }, icon: "download" },
  "Open Thread": { get label() { return t("ui.open-thread"); }, icon: "message-square-text" },
  "Pin": { get label() { return t("ui.pin-to-channel"); }, icon: "pin" },
  "Unpin": { get label() { return t("ui.unpin"); }, icon: "pin" },
  "Edit": { get label() { return t("ui.edit"); }, icon: "pencil" },
  "Hide for Me": { get label() { return t("ui.hide-for-me"); }, icon: "eye-off" },
  "Save": { get label() { return t("ui.save-message"); }, icon: "bookmark" },
  "Unsave": { get label() { return t("ui.remove-from-saved"); }, icon: "bookmark" },
  "Remind Me": { get label() { return t("ui.remind-me"); }, icon: "bell" },
  "Copy text": { get label() { return t("ui.copy-text"); }, icon: "copy" },
  "More": { get label() { return t("ui.more"); }, icon: "ellipsis" },
  "Report Message": { get label() { return t("ui.report-message"); }, icon: "flag" },
  "Delete": { get label() { return t("ui.delete"); }, icon: "trash-2", danger: true },
  "Delete (Moderator)": { get label() { return t("ui.delete-as-moderator"); }, icon: "trash-2", danger: true },
  "Rename Thread": { get label() { return t("ui.rename-thread-2"); }, icon: "pencil" },
  "Archive Thread": { get label() { return t("ui.archive-thread-2"); }, icon: "archive" },
  "Unarchive Thread": { get label() { return t("ui.unarchive-thread"); }, icon: "archive" },
  "Lock Thread": { get label() { return t("ui.lock-thread-2"); }, icon: "lock" },
  "Unlock Thread": { get label() { return t("ui.unlock-thread"); }, icon: "lock" },
};

function swipeAxis(dx, dy, lean = SWIPE.lean) {
  if (Math.abs(dx) < SWIPE.slop && Math.abs(dy) < SWIPE.slop) return null;
  return Math.abs(dx) > Math.abs(dy) * lean ? "x" : "y";
}

function replyPull(dx) {
  if (!Number.isFinite(dx)) return dx > 0 ? SWIPE.replyMax : dx < 0 ? -SWIPE.replyMax : 0;
  return Math.max(-SWIPE.replyMax, Math.min(SWIPE.replyMax, dx));
}

function replyCommits(dx) {
  return Math.abs(dx) >= SWIPE.replyAt;
}

function messageSwipePull(dx, canReply) {
  return replyPull(canReply ? dx : Math.min(0, dx));
}

function messageSwipeAction(dx) {
  if (dx > 0) return "reply";
  if (dx < 0) return "menu";
  return null;
}

function edgeBackStarts(x) {
  return x <= SWIPE.edge;
}

function rowPull(dx, wasOpen) {
  const raw = (wasOpen ? SWIPE.rowOpen : 0) + dx;
  if (raw <= 0) return 0;
  if (raw <= SWIPE.rowOpen) return raw;
  return Math.min(SWIPE.rowOpen + 18, SWIPE.rowOpen + (raw - SWIPE.rowOpen) * 0.25);
}

function rowStaysOpen(pull, wasOpen, speed = 0) {
  if (speed <= -SWIPE.rowFlick) return false;
  if (speed >= SWIPE.rowFlick && pull >= SWIPE.rowFlickAt) return true;
  return wasOpen ? pull > SWIPE.rowOpen - SWIPE.rowAt : pull >= SWIPE.rowAt;
}

function sheetPull(dy) {
  return Math.max(0, Number(dy) || 0);
}

function trayOpens(dy) {
  return (Number(dy) || 0) <= -TRAY_PULL;
}

function sheetCloses(dy) {
  return sheetPull(dy) >= SWIPE.sheetAt;
}

const BAR_EXTRAS = [
  { id: "emoji", get label() { return t("ui.emoji"); }, icon: "smile", press: "emoji-btn" },
  { id: "style", get label() { return t("ui.text-style"); }, icon: "a-large-small", press: "text-style-btn" },
];

function sheetRow(title) {
  const known = SHEET_ROWS[title] || SHEET_ROWS[englishFor(String(title || "").trim())];
  if (known) return known;
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
  if (view === "dm") return presence || t("ui.direct-message");
  if (group) return t("ui.group-chat");
  return serverName || "";
}

const NOTIFY_PICKS = [
  { level: "all", get label() { return t("ui.all-messages"); }, icon: "bell-ring" },
  { level: "mentions", get label() { return t("ui.mentions-and-dms"); }, icon: "at-sign" },
  { level: "none", get label() { return t("ui.nothing"); }, icon: "bell-off" },
];

function chatBanner({ view, channel }) {
  return view === "server" && channel && channel.banner ? String(channel.banner) : "";
}

function faceTapAuthor(message) {
  return message && message.author_id ? { id: message.author_id, name: message.author_name || "" } : null;
}

function layerStep(key, open, held, entry) {
  const onStep = Boolean(entry && entry[key]);
  if (open) return onStep ? "keep" : "push";
  return held && onStep ? "back" : "none";
}

function inviteWarningParts(text) {
  const words = String(text || "").trim();
  const cut = words.indexOf(". ");
  return cut < 0 ? { lead: words, rest: "" } : { lead: words.slice(0, cut + 1), rest: words.slice(cut + 1) };
}

function trayTap(wasOpen, place) {
  if (!wasOpen || place === "tray" || place === "outside") return "pass";
  return place === "key" ? "stay" : "close";
}

function trayNeedsStep(entry) {
  return Boolean(entry && entry.ptSheet === "pt-voice-open" && !entry.ptTray);
}

const ROOM_KEYS = [
  { id: "more", get label() { return t("ui.more"); }, icon: "ellipsis" },
  { id: "speaker", get label() { return t("ui.speaker"); }, icon: "volume-2" },
  { id: "mic", get label() { return t("ui.mic"); }, icon: "mic" },
  { id: "leave", get label() { return t("ui.leave"); }, icon: "phone-off" },
];
const ROOM_MENU = [
  { id: "sounds", get label() { return t("settings.tab.sounds"); }, icon: "music" },
  { id: "camera", get label() { return t("settings.tab.camera"); }, icon: "video" },
  { id: "deafen", get label() { return t("ui.deafen"); }, icon: "headphones" },
  { id: "share", get label() { return t("ui.share-screen"); }, icon: "monitor-up" },
];
const ROOM_SWITCHES = ["camera", "deafen"];
const ROOM_LAMPS = ["more", "speaker", "mic"];
const SOUND_ICONS = { speaker: "volume-2", phone: "smartphone", bluetooth: "bluetooth", wired: "headphones" };
const SIGNAL_WORDS = { get good() { return t("ui.signal.good"); }, get ok() { return t("ui.signal.fair"); }, get bad() { return t("ui.signal.poor"); } };

function roomTiles(channel, me = {}) {
  if (!channel) return [];
  const muted = channel.voice_muted || [];
  const deafened = channel.voice_deafened || [];
  return roomPeople(channel).map((person, i) => {
    const you = Boolean(me.userId && person.id === me.userId);
    return {
      id: person.id,
      name: person.name,
      label: you ? t("settings.group.you") : person.name,
      you,
      muted: you ? Boolean(me.muted) : Boolean(muted[i]),
      deafened: you ? Boolean(me.deafened) : Boolean(deafened[i]),
    };
  });
}

function duoTiles(name, me = {}) {
  return [
    { id: me.otherId || null, name: name || t("ui.call"), label: name || t("ui.call"), you: false, muted: false, deafened: false },
    { id: me.userId || null, name: me.name || t("settings.group.you"), label: t("settings.group.you"), you: true, muted: Boolean(me.muted), deafened: Boolean(me.deafened) },
  ];
}

function roomHeading({ serverName, count, call }) {
  if (call) return t("ui.voice-call");
  const here = count === 1 ? t("ui.room.here.one") : t("ui.room.here.many", { count });
  return serverName ? `${serverName} \u00B7 ${here}` : here;
}

function roomStreams(server, channelId, me = {}) {
  if (!server || !channelId) return [];
  const room = liveRooms([server], me).find(found => found.channelId === channelId);
  if (!room) return [];
  return room.streams
    .filter(stream => !stream.mine && stream.userId !== me.userId)
    .map(stream => ({ user: stream.userId, name: stream.name, title: t("ui.name-s-screen", { name: stream.name }), watching: stream.userId === me.watching }));
}

function signalWord(quality) {
  return SIGNAL_WORDS[quality] || "";
}

function roomKeyState(id, voice = {}) {
  if (id === "mic" && voice.hand) {
    const word = voice.hand.raised ? t("ui.lower-hand") : t("ui.raise-hand");
    return { down: Boolean(voice.hand.raised), mode: "hand", icon: "hand", label: word, word };
  }
  if (id === "mic" && voice.talk && !voice.muted) return { down: false, mode: "talk", held: Boolean(voice.talking), icon: "mic", label: t("ui.hold-to-talk"), word: t("ui.hold-to-talk") };
  if (id === "mic") return { down: Boolean(voice.muted), icon: voice.muted ? "mic-off" : "mic", label: voice.muted ? t("ui.unmute") : t("ui.mute") };
  if (id === "deafen") return { down: Boolean(voice.deafened), icon: voice.deafened ? "headphone-off" : "headphones", label: voice.deafened ? t("ui.undeafen") : t("ui.deafen") };
  if (id === "camera") return { down: false, off: !voice.camera, on: Boolean(voice.camera), icon: voice.camera ? "video" : "video-off", label: voice.camera ? t("ui.turn-off-camera") : t("ui.turn-on-camera-2") };
  if (id === "sounds") return { down: Boolean(voice.sounds), icon: "music", label: t("ui.soundboard") };
  if (id === "share") return { down: false, on: Boolean(voice.sharing), icon: "monitor-up", label: voice.sharing ? t("ui.stop-sharing-your-screen") : t("ui.share-your-screen-2") };
  if (id === "speaker") return speakerKeyState(voice.sound);
  if (id === "more") return { down: Boolean(voice.more), lit: roomMenuBusy(voice), icon: voice.more ? "x" : "ellipsis", label: t("ui.more-voice-controls") };
  return { down: false, icon: "phone", label: t("ui.hang-up") };
}

function roomControl(id) {
  return ROOM_KEYS.find(key => key.id === id) || ROOM_MENU.find(item => item.id === id);
}

function roomMenuBusy(voice = {}) {
  return Boolean(voice.deafened || voice.camera || voice.sharing);
}

function soundIcon(place) {
  return SOUND_ICONS[place] || "volume-2";
}

function soundWord(place, name) {
  if (place === "speaker") return t("ui.speaker");
  if (place === "phone") return t("ui.sound-phone");
  return name || t("ui.sound-headphones");
}

function speakerKeyState(route) {
  const now = route ? route.now : "speaker";
  const word = soundWord(now, route && route.name);
  return { down: false, lit: now !== "phone", icon: soundIcon(now), label: t("ui.sound-output-is", { place: word }), word };
}

function soundNeedsList(route) {
  return Boolean(route) && route.choices.length > 2;
}

function soundTapGoesTo(route) {
  const next = route && route.choices.find(choice => !choice.on);
  return next ? next.id : "speaker";
}

function soundCanMove(route) {
  return Boolean(route) && route.choices.length > 1;
}

function canOfferShare({ mobileApp, canShare, iPhone }) {
  return !iPhone && (!mobileApp || Boolean(canShare));
}

function roomKeyOffered(id, { mobileApp, canPickSound }) {
  if (id === "speaker") return Boolean(mobileApp && canPickSound);
  return true;
}

function roomMenuOffered(id, { mobileApp, finePointer, canShare, iPhone }) {
  if (id !== "share") return true;
  return mobileApp ? canOfferShare({ mobileApp, canShare, iPhone }) : Boolean(finePointer);
}

function roomTileStreams(streams, me = {}) {
  if (!me.sharing) return streams || [];
  return [{ user: me.userId, name: me.name || "", title: t("ui.your-screen"), short: t("settings.group.you"), own: true, watching: false, viewers: Math.max(0, Number(me.viewers) || 0) }, ...(streams || [])];
}

function roomCameraTiles(cameraIds, people, me = {}) {
  const on = new Set(cameraIds || []);
  const showing = person => person.id && (person.id === me.userId ? Boolean(me.camera) : on.has(person.id));
  const tiles = (people || []).filter(showing).map(person => {
    const own = person.id === me.userId;
    return { user: person.id, name: person.name, title: own ? t("settings.group.you") : person.name, own, camera: true, rear: own && me.facing === "environment", canFlip: own && Boolean(me.canFlip) };
  });
  return [...tiles.filter(tile => tile.own), ...tiles.filter(tile => !tile.own)];
}

function shareViewers(text, hidden = false) {
  return hidden ? 0 : Math.max(0, parseInt(text, 10) || 0);
}

function roomStage(channel, tiles) {
  if (!channel || channel.kind !== "stage") return { tiles: tiles || [], audience: [] };
  const speakers = channel.speakers || [];
  const hands = channel.hands || [];
  const rank = id => (hands.includes(id) ? hands.indexOf(id) : hands.length);
  const audience = (tiles || [])
    .filter(tile => !speakers.includes(tile.id))
    .map((tile, index) => ({ ...tile, hand: hands.includes(tile.id), index }))
    .sort((a, b) => rank(a.id) - rank(b.id) || a.index - b.index);
  return { tiles: (tiles || []).filter(tile => speakers.includes(tile.id)), audience };
}

const ROOM_MEMBER_ACTION_WORDS = { move: () => t("ui.move-to-another-room"), disconnect: () => t("ui.disconnect-from-voice"), kick: name => t("ui.kick-name", { name }), ban: name => t("ui.ban-name", { name }) };

function roomMemberActions(server, { targetId, myId, roomId } = {}) {
  if (!server || !targetId || targetId === myId || targetId === server.owner) return [];
  if (!(server.members || []).some(member => member.id === targetId)) return [];
  const actions = [];
  if (server.can_move) {
    const rooms = (server.channels || []).filter(channel => (channel.kind === "voice" || channel.kind === "stage") && channel.id !== roomId && channel.can_view !== false);
    if (rooms.length) actions.push({ kind: "move", rooms });
    actions.push({ kind: "disconnect" });
  }
  if (server.can_kick) actions.push({ kind: "kick" });
  if (server.can_ban) actions.push({ kind: "ban" });
  return actions;
}

function roomMemberActionWords(kind, name) {
  return ROOM_MEMBER_ACTION_WORDS[kind](name);
}

function audienceLabel(count) {
  return t("ui.audience-count", { count });
}

function roomSignature(tiles, streams, extra = "") {
  return [
    tiles.map(tile => `${tile.id}:${tile.name}:${tile.muted ? 1 : 0}${tile.deafened ? 1 : 0}`).join(","),
    streams.map(stream => `${stream.camera ? "c" : "s"}${stream.user}:${stream.watching ? 1 : 0}${stream.rear ? 1 : 0}${stream.canFlip ? 1 : 0}`).join(","),
    extra,
  ].join("|");
}

const MIRROR_FRAME_MS = 33;

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

function watchPlace(voiceServer, withServer = true) {
  if (!voiceServer) return "";
  if (voiceServer.call) return t("ui.call-with", { name: voiceServer.name });
  return [voiceServer.channelName, withServer ? voiceServer.name : ""].filter(Boolean).join(" \u00B7 ");
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    PHONE_TABS,
    HORIZON,
    HORIZON_SPRINGS,
    STACK_MAX,
    TILE_NAME,
    tileNameWidth,
    WAIT_CARDS,
    waitCards,
    waitMoreWords,
    listAge,
    listLabel,
    unreadChats,
    ROOM_LAMPS,
    TRAY_PULL,
    trayOpens,
    canOfferShare,
    roomKeyOffered,
    roomMenuOffered,
    roomControl,
    roomMenuBusy,
    soundIcon,
    soundWord,
    soundNeedsList,
    soundTapGoesTo,
    soundCanMove,
    roomFaces,
    horizonY,
    horizonPath,
    horizonLift,
    springStep,
    tabForConversation,
    phoneTabAfter,
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
    messageSwipePull,
    messageSwipeAction,
    replyPull,
    replyCommits,
    edgeBackStarts,
    rowPull,
    rowStaysOpen,
    sheetPull,
    sheetCloses,
    BAR_EXTRAS,
    sheetRow,
    quickReactions,
    chatSubtitle,
    ROOM_KEYS,
    ROOM_MENU,
    ROOM_SWITCHES,
    trayTap,
    NOTIFY_PICKS,
    chatBanner,
    layerStep,
    faceTapAuthor,
    inviteWarningParts,
    trayNeedsStep,
    roomTiles,
    duoTiles,
    roomHeading,
    roomStreams,
    signalWord,
    roomKeyState,
    roomSignature,
    WATCH_STEPS,
    watchStack,
    STREAM_TOOLS_MS,
    streamToolsStay,
    roomTileStreams,
    roomCameraTiles,
    shareViewers,
    roomStage,
    audienceLabel,
    roomMemberActions,
    roomMemberActionWords,
    MIRROR_FRAME_MS,
    streamBubbles,
    watchPlace,
  };
}
