import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { VOICE_ROOM, VOICE_ROOM_TILE, VOICE_ROOM_TOOLS, voiceRoomCols, voiceRoomLayout, voiceRoomPeople, voiceRoomItems, voiceRoomQuality, voiceRoomSub, voiceRoomSoundsPlace, voiceRoomHint, VOICE_ROOM_REST, VOICE_ROOM_ENDED, voiceRoomLeftWords, voiceRoomAway, voiceRoomGhostTiles, mountStudioVoiceRoom } = require("../scripts/studio-voice-room.js");
const { callTiles } = require("../scripts/stream-view.js");
const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const css = read("../styles/studio.css");
const app = read("../scripts/app.js");

const DESK = 'html[data-look="studio"]:not([data-fit="slim"])';
const people = count => Array.from({ length: count }, (_, i) => ({ key: `person:${i}`, kind: "person" }));
const streams = count => Array.from({ length: count }, (_, i) => ({ key: `screen:${i}`, kind: "stream" }));
const cameras = count => Array.from({ length: count }, (_, i) => ({ key: `camera:${i}`, kind: "camera" }));
const box = spot => [Math.round(spot.x), Math.round(spot.y), Math.round(spot.w), Math.round(spot.h)];
const rows = placed => [...new Set(placed.map(spot => Math.round(spot.y)))].map(y => placed.filter(spot => Math.round(spot.y) === y).length);

function fakeNode(tag) {
  const node = {
    tag,
    id: "",
    className: "",
    title: "",
    type: "",
    attrs: {},
    children: [],
    after: null,
    setAttribute(name, value) { this.attrs[name] = value; },
    appendChild(child) { this.children.push(child); return child; },
    append(...kids) { this.children.push(...kids); },
  };
  return node;
}

function fakeApp(look, withMain = true) {
  const chatHead = fakeNode("header");
  const main = fakeNode("main");
  main.querySelector = selector => (selector === ".chat-header" ? chatHead : null);
  main.after = room => { main.next = room; };
  const app = { querySelector: selector => (selector === ".main" && withMain ? main : null) };
  const ids = new Map();
  return {
    main,
    chatHead,
    documentElement: { dataset: look ? { look } : {} },
    createElement: fakeNode,
    getElementById: id => (id === "app" ? app : id === "voice-room" ? main.next || null : ids.get(id) || null),
  };
}

test("two streams, one camera and four people draw as the guide's four quarters", () => {
  const plan = voiceRoomLayout({ width: 1033, height: 614, items: [...streams(2), ...cameras(1), ...people(4)] });
  assert.equal(plan.mode, "quarters");
  assert.deepEqual(plan.placed.slice(0, 3).map(box), [[0, 0, 508, 286], [525, 0, 508, 286], [0, 328, 508, 286]]);
  assert.deepEqual(plan.placed.slice(0, 3).map(spot => spot.form), ["picture", "picture", "picture"]);
  const seats = plan.placed.slice(3);
  assert.deepEqual(seats.map(spot => spot.form), ["seat", "seat", "seat", "seat"]);
  assert.deepEqual(seats.map(box), [[525, 328, 248, 137], [785, 328, 248, 137], [525, 477, 248, 137], [785, 477, 248, 137]]);
  assert.equal(Math.round(plan.bottom), 614);
});

test("a room with nobody streaming grows from one tile to compact cards", () => {
  assert.deepEqual([1, 2, 4, 5, 9, 10, 12, 13, 40].map(voiceRoomCols), [1, 2, 2, 3, 3, 4, 4, 5, 5]);
  const one = voiceRoomLayout({ width: 1033, height: 614, items: people(1) });
  assert.equal(one.mode, "people");
  assert.deepEqual(box(one.placed[0]), [277, 172, 480, 270]);
  assert.equal(one.placed[0].form, "tile");
  assert.deepEqual(rows(voiceRoomLayout({ width: 1033, height: 614, items: people(4) }).placed), [2, 2]);
  const five = voiceRoomLayout({ width: 1033, height: 614, items: [...cameras(1), ...people(4)] });
  assert.deepEqual(rows(five.placed), [3, 2]);
  assert.equal(five.placed[0].form, "picture");
  const last = five.placed.slice(3);
  assert.equal(Math.round((last[0].x + last[1].x + last[1].w) / 2), 517);
  assert.deepEqual(rows(voiceRoomLayout({ width: 1033, height: 614, items: people(12) }).placed), [4, 4, 4]);
  const crowd = voiceRoomLayout({ width: 1033, height: 614, items: people(25) });
  assert.equal(crowd.mode, "compact");
  assert.deepEqual(rows(crowd.placed), [5, 5, 5, 5, 5]);
  assert.ok(crowd.placed.every(spot => spot.form === "compact" && Math.round(spot.w) === 194));
  assert.ok(crowd.bottom <= 614);
  const tight = voiceRoomLayout({ width: 904, height: 520, items: people(25) });
  assert.equal(rows(tight.placed)[0], 4);
  assert.ok(tight.placed.every(spot => spot.w >= VOICE_ROOM.compactMin));
  assert.ok(voiceRoomLayout({ width: 1033, height: 614, items: people(40) }).bottom > 614);
  for (const spot of [...one.placed, ...five.placed]) assert.ok(Math.abs(spot.w / spot.h - VOICE_ROOM_TILE) < 0.01);
});

test("more pictures or people than the quarters hold puts the streams across the top 58%", () => {
  const plan = voiceRoomLayout({ width: 1300, height: 614, items: [...streams(1), ...people(5)] });
  assert.equal(plan.mode, "top");
  const [stream, ...cards] = plan.placed;
  assert.equal(stream.form, "picture");
  assert.equal(Math.round(stream.h), Math.round(614 * VOICE_ROOM.streamShare));
  assert.equal(rows(cards).length, 1);
  assert.ok(cards.every(card => card.form === "compact" && card.w >= VOICE_ROOM.cardMin && card.y >= stream.y + stream.h + VOICE_ROOM.underGap));
  const busy = voiceRoomLayout({ width: 1033, height: 614, items: [...streams(4), ...cameras(2), ...people(6)] });
  const top = busy.placed.filter(spot => spot.form === "picture");
  assert.deepEqual(top.map(spot => spot.key), ["screen:0", "screen:1", "screen:2"]);
  const under = busy.placed.filter(spot => spot.form !== "picture");
  assert.deepEqual(under.slice(0, 3).map(spot => [spot.key, spot.form]), [["camera:0", "small"], ["camera:1", "small"], ["screen:3", "compact"]]);
  assert.ok(under.slice(2).every(spot => spot.w >= VOICE_ROOM.cardMin));
  const narrow = voiceRoomLayout({ width: 1033, height: 614, items: [...streams(1), ...people(5)] });
  assert.deepEqual(rows(narrow.placed.slice(1)), [4, 1]);
  assert.ok(narrow.bottom <= 614.5);
  assert.deepEqual(rows(voiceRoomLayout({ width: 1600, height: 614, items: [...streams(1), ...people(8)] }).placed.slice(1)), [4, 4]);
  assert.equal(voiceRoomLayout({ width: 1033, height: 614, items: [] }).mode, "empty");
});

test("an expanded stream fills the pane and everything else is one strip under it", () => {
  const items = [...streams(2), ...cameras(1), ...people(2)];
  const plan = voiceRoomLayout({ width: 1033, height: 614, items, expanded: "screen:0" });
  assert.equal(plan.mode, "expanded");
  assert.deepEqual([plan.placed[0].key, plan.placed[0].form], ["screen:0", "big"]);
  assert.deepEqual(box(plan.placed[0]), [109, 0, 816, 459]);
  const strip = plan.placed.slice(1);
  assert.deepEqual(strip.map(spot => spot.key), ["screen:1", "camera:0", "person:0", "person:1"]);
  assert.ok(strip.every(spot => spot.form === "mini" && Math.round(spot.y) === 485 && Math.round(spot.h) === 129));
  assert.ok(plan.bottom <= 614.5);
  const alone = voiceRoomLayout({ width: 1033, height: 614, items: streams(1), expanded: "screen:0" });
  assert.deepEqual(box(alone.placed[0]), [0, 0, 1033, 581]);
  assert.equal(voiceRoomLayout({ width: 1033, height: 614, items, expanded: "screen:9" }).mode, "quarters");
});

test("smaller windows pass their own gaps and the plan keeps inside the pane", () => {
  const tight = { ...VOICE_ROOM, gap: 12, keyGap: 36, seatGap: 8 };
  const plan = voiceRoomLayout({ width: 844, height: 468, items: [...streams(2), ...cameras(1), ...people(4)] }, tight);
  assert.equal(Math.round(plan.placed[1].x - plan.placed[0].x - plan.placed[0].w), 12);
  assert.equal(Math.round(plan.placed[2].y - plan.placed[0].y - plan.placed[0].h), 36);
  assert.ok(plan.bottom <= 468.5);
  assert.deepEqual(voiceRoomLayout({ width: 0, height: 0, items: people(3) }), { mode: "empty", placed: [], bottom: 0 });
});

test("a card shows deafened over muted over speaking, and marks you, Live and cameras", () => {
  const list = voiceRoomPeople({
    userIds: ["a", "b", "c", "me"],
    names: ["ferris", "quill", "juno", "Commy Commander"],
    muted: [false, false, true, true],
    deafened: [false, false, false, true],
    speaking: ["b", "c"],
    shares: [{ user: "a", channel: "room" }, { user: "b", channel: "other" }],
    cameras: ["c"],
    meId: "me",
    channelId: "room",
  });
  assert.deepEqual(list.map(person => person.state), ["", "speaking", "muted", "deafened"]);
  assert.deepEqual(list.map(person => person.live), [true, false, false, false]);
  assert.deepEqual(list.map(person => person.camera), [false, false, true, false]);
  assert.deepEqual(list.map(person => person.me), [false, false, false, true]);
  assert.deepEqual(voiceRoomPeople({}), []);
});

test("streams come first, then cameras, then a card for everyone without a camera", () => {
  const tiles = callTiles({ userIds: ["a", "b", "me"], names: ["ferris", "nova", "me"], cameras: ["b"], shares: [{ user: "a", name: "ferris" }, { user: "me", name: "me" }], meId: "me" });
  const items = voiceRoomItems(tiles, [{ id: "a", camera: false }, { id: "b", camera: true }, { id: "me", camera: false }]);
  assert.deepEqual(items, [
    { key: "screen:a", kind: "stream" },
    { key: "screen:me", kind: "stream" },
    { key: "camera:b", kind: "camera" },
    { key: "person:a", kind: "person" },
    { key: "person:me", kind: "person" },
  ]);
  assert.deepEqual(voiceRoomItems(null, null), []);
});

test("the header's second slot is the room status, else the server name", () => {
  assert.deepEqual(voiceRoomSub({ status: " Ranked grind ", server: "Pixel Pioneers" }), { text: "Ranked grind", kind: "status" });
  assert.deepEqual(voiceRoomSub({ status: "", server: "Pixel Pioneers" }), { text: "Pixel Pioneers", kind: "server" });
  assert.deepEqual(voiceRoomSub({}), { text: "", kind: "server" });
});

test("your stream's chip names the quality chosen at the start and its readout", () => {
  assert.deepEqual(voiceRoomQuality({ quality: "smooth", lines: 1080, fps: 60 }), { word: "Smooth", rest: ": best for games", label: "Smooth: best for games", readout: "Smooth, 1080p, 60 fps" });
  assert.equal(voiceRoomQuality({ quality: "sharp" }).label, "Sharp: best for text");
  assert.equal(voiceRoomQuality({ quality: "sharp", lines: 1440 }).readout, "Sharp, 1440p");
  assert.equal(voiceRoomQuality({}).word, "Balanced");
  assert.equal(voiceRoomQuality({ quality: "constructor" }).word, "Balanced");
});

test("the room is built once, hidden, after the chat pane, and only in the new look", () => {
  assert.equal(mountStudioVoiceRoom(fakeApp(null), null), null);
  assert.equal(mountStudioVoiceRoom(fakeApp("studio", false), null), null);
  const doc = fakeApp("studio");
  const icons = [];
  const room = mountStudioVoiceRoom(doc, (name, size) => {
    icons.push(name);
    return fakeNode("svg");
  });
  assert.equal(doc.main.next, room);
  assert.equal(room.id, "voice-room");
  assert.equal(room.className, "voice-room hidden");
  const [head, grid] = room.children;
  assert.deepEqual(head.children.map(child => child.id || child.className), ["vr-fold", "vr-title", "vr-sub", "vr-clock", "vr-tools"]);
  assert.deepEqual(head.children[4].children.map(child => [child.id, child.title]), [["vr-invite", "Invite people"], ["vr-activities", "Activities"], ["vr-full", "Full screen"], ["vr-chat", "Hide chat"]]);
  assert.deepEqual(VOICE_ROOM_TOOLS.map(tool => tool.icon), ["user-plus", "gamepad-2", "maximize-2", "message-square"]);
  assert.deepEqual(icons, ["panel-left", "volume-2", "user-plus", "gamepad-2", "maximize-2", "message-square", "x"]);
  assert.equal(grid.id, "vr-grid");
  assert.equal(grid.children[0].id, "vr-cards");
  assert.deepEqual(doc.chatHead.children.map(child => [child.id, child.className]), [["vr-chat-x", "icon-btn vr-wing-x hidden"]]);
  assert.equal(mountStudioVoiceRoom(doc, null), room);
  assert.equal(doc.chatHead.children.length, 1);
});

test("the room script loads after the dock and before the app", () => {
  const dock = html.indexOf('<script src="scripts/studio-dock.js"></script>');
  const room = html.indexOf('<script src="scripts/studio-voice-room.js"></script>');
  assert.ok(dock > 0 && room > dock && html.indexOf('<script src="scripts/app.js"></script>') > room);
  assert.equal(html.includes('id="voice-room"'), false);
});

test("the room view only opens on the desktop fit and closes when you pick a text channel or leave the room", () => {
  assert.match(app, /function openVoiceRoom\(\) \{\n  const room = document\.getElementById\("voice-room"\);\n  if \(!room \|\| !studioDesk\(\) \|\| !callChannelHere\(\)\) return false;/);
  assert.match(app, /if \(!channel \|\| state\.view !== "server" \|\| !studioDesk\(\)\) return closeVoiceRoom\(\);/);
  assert.match(app, /if \(switched\) closeVoiceRoom\(\);/);
  assert.match(app, /state\.historyAnchor = null;\n    closeVoiceRoom\(\);\n    selectChannel\(channel\);/);
  assert.match(app, /phoneShowChat\(\);\n  if \(openVoiceRoom\(\)\) return;/);
  assert.match(app, /\$\("messages"\)\.before\(\$\("video-pane"\), \$\("cam-grid"\), \$\("activity-panel"\)\);/);
});

test("the room's styles stay off the phone fit and the old look", () => {
  const mine = css.split("\n").filter(line => /\.vr-|\.voice-room/.test(line) && line.includes("{") && !line.trim().startsWith("@"));
  assert.ok(mine.length > 80);
  for (const line of mine) assert.ok(line.trim().startsWith(DESK) || line.trim().startsWith('html[data-look="studio"][data-fit="compact"]'), line);
  assert.match(css, /#app\.vr-open \.voice-panel \{[^}]*position: fixed;/);
  assert.match(css, /#app\.vr-open\.vr-folded \{ grid-template-columns: 0 minmax\(0, 1fr\)/);
  assert.match(css, /\.vr-keys \{[^}]*bottom: -17px;/);
  assert.match(css, /\.vr-dots i \{[^}]*animation: studio-room-dot/);
  assert.equal(/\.vr-[a-z-]+[^{]*\{[^}]*spinner/.test(css), false);
});

test("the soundboard pane rises out of the wide dock where the guide draws it", () => {
  assert.deepEqual(voiceRoomSoundsPlace({ left: 82, top: 796 }, { width: 396, height: 176 }, { width: 1440 }), { left: 344, top: 608 });
  assert.deepEqual(voiceRoomSoundsPlace({ left: 37, top: 656 }, { width: 380, height: 176 }, { width: 1180 }), { left: 64, top: 468 });
  assert.deepEqual(voiceRoomSoundsPlace({ left: 900, top: 100 }, { width: 396, height: 176 }, { width: 1440 }), { left: 1036, top: 8 });
  assert.match(app, /dressSoundboardPop\(pop\);\n  pop\.classList\.remove\("hidden"\);\n  placePopup\(pop, \$\("vp-soundboard"\), "above"\);\n  placeSoundboardPop\(pop\);/);
});

test("a soundboard pad is its name, with the sound's own emoji when it has one, on the desktop fit", () => {
  assert.match(app, /function soundPadMark\(emoji\) \{\n  if \(emoji\) return soundEmojiNode\(emoji\);\n  return studioDesk\(\) \? null : svgIcon\("soundboard", 14\);\n\}/);
  assert.equal((app.match(/const mark = soundPadMark\(/g) || []).length, 2);
  assert.match(app, /const addPad = canAdd && studioDesk\(\) \? soundAddPad\(\) : null;/);
  assert.match(app, /for \(const node of pop\.querySelectorAll\("\.sb-line, \.sb-x"\)\) node\.remove\(\);\n  if \(!studioDesk\(\)\) return;/);
  for (const part of [".sb-sound", ".sb-line", ".sb-x", ".sb-section", ".sb-sound.hit::after"]) assert.ok(css.includes(`${DESK} .soundboard-pop ${part}`), part);
  for (const part of [".pt-tray-grab", ".pt-tray-line", ".pt-tray-add"]) assert.ok(css.includes(`${DESK} .soundboard-pop ${part}`), part);
  assert.match(app, /el\("small", "sb-line", "Played to the whole room\."\)/);
  assert.match(app, /const alone = soundGroupsAlone\(grid\);\n  grid\.classList\.toggle\("sb-one", alone\);\n  if \(alone && addPad\) grid\.appendChild\(addPad\);/);
  assert.ok(css.includes(`${DESK} .soundboard-pop .sb-grid.sb-one .sb-section,`));
});

test("in the narrow chat the hint drops the channel name when it does not fit, and cards stack", () => {
  assert.equal(voiceRoomHint("Message #welcome", 118, 110), "Message");
  assert.equal(voiceRoomHint("Message #welcome", 118, 118), "Message #welcome");
  assert.match(app, /if \(voiceRoom\.hint !== null && input\.placeholder === "Message"\) input\.placeholder = voiceRoom\.hint;/);
  assert.ok(!css.includes("#app.vr-open .main .invite-card"));
  assert.match(css, /#app\.vr-open \.main \.msg [^{]*> \.chat-image \{ max-width: 100%; max-height: 160px; border-radius: 14px; \}/);
});

test("other people's stream tiles carry Draw, Point, eraser, volume, expand and a round x, in that order", () => {
  const keys = app.slice(app.indexOf('const draw = voiceRoomTileKey("latch vr-draw"'), app.indexOf("pad.onpointercancel = finishVoiceRoomPen;"));
  const listed = keys.slice(keys.indexOf("keys.append("));
  assert.ok(listed.indexOf("draw,") < listed.indexOf("vr-point") && listed.indexOf("vr-point") < listed.indexOf("vr-wipe") && listed.indexOf("vr-wipe") < listed.indexOf("volume,") && listed.indexOf("volume,") < listed.indexOf("vr-big") && listed.indexOf("vr-big") < listed.indexOf("vr-x"));
  assert.match(app, /const allowed = live && ink\.allowed\.get\(t\.user\) === true;\n  if \(!allowed && voiceRoom\.pen\.user === t\.user\) dropVoiceRoomTool\(\);/);
  assert.match(app, /for \(const part of \["\.vr-draw", "\.vr-point", "\.vr-wipe", "\.vr-allowed"\]\) tile\.querySelector\(part\)\.classList\.toggle\("hidden", !allowed\);/);
  assert.match(app, /el\("span", null, "The room can draw on this"\)/);
});

test("drawings land on the stream's own tile, and a key that is on is lit, never the grey key", () => {
  assert.match(app, /function onStreamInk\(news\) \{\n  onVoiceRoomInk\(news\);/);
  assert.match(app, /function onVoiceRoomInk\(news\) \{\n  if \(!voiceRoom\.open \|\| !news \|\| !news\.sharer\) return;\n  inkAdd\(voiceRoomInkStore\(news\.sharer\), news, performance\.now\(\), news\.sharer\);/);
  assert.match(app, /if \(!news\.on\) voiceRoom\.ink\.delete\(news\.sharer\);/);
  assert.match(app, /dropVoiceRoomTool\(\);\n  voiceRoom\.ink\.clear\(\);\n  voiceRoom\.drawers = \[\];\n  closeVoiceRoomPop\(\);/);
  assert.ok(!/classList\.toggle\("is-down"/.test(app.slice(app.indexOf("function dressVoiceRoomTile"), app.indexOf("function placeVoiceRoom"))));
  assert.ok(css.includes(`${DESK} .vr-keys .key.latch.is-on > .led { background: var(--speaking); }`));
  assert.ok(css.includes(`${DESK}:not([data-theme="light"]) .vr-keys .key.is-on {`));
  assert.ok(css.includes(`${DESK} .vr-ink { position: absolute; inset: 0; width: 100%; height: 100%; border-radius: inherit; pointer-events: none; }`));
});

test("volume opens a small glass popover with a slider, and Allow drawing lists who is drawing", () => {
  assert.match(app, /range\.max = String\(STREAM_VOLUME_MAX_PERCENT\);/);
  assert.match(app, /range\.onchange = \(\) => setStreamVolume\(Number\(range\.value\)\);/);
  assert.match(app, /function onVoiceRoomAllow\(anchor\) \{\n  if \(!voiceRoom\.drawers\.length\) return setStreamInk\(!state\.shareInk\);\n  showVoiceRoomPop\(anchor, "drawing", voiceRoomDrawParts\(\)\);/);
  assert.match(app, /stop\.onclick = \(\) => invoke\("block_stream_ink", \{ userId: drawer\.id \}\)/);
  assert.match(app, /renderStageDrawers\(own\.drawers\);\n      setVoiceRoomDrawers\(own\.drawers\);/);
  for (const part of [".vr-pop {", ".vr-slider input[type=\"range\"]::-webkit-slider-thumb {", ".vr-pop-row {", ".vr-pop-pen {"]) assert.ok(css.includes(`${DESK} ${part}`), part);
});

test("after hang up the dock says you left, in one line that is never cut", () => {
  assert.deepEqual(voiceRoomLeftWords("General", () => true), { line: "Not in voice", sub: "You left General. It noticed." });
  assert.deepEqual(voiceRoomLeftWords("General", () => false), { line: "Not in voice", sub: "You left General." });
  assert.equal(voiceRoomLeftWords(" Late night games ").sub, "You left Late night games. It noticed.");
  const asked = [];
  voiceRoomLeftWords("General", text => asked.push(text) > 0);
  assert.deepEqual(asked, ["You left General. It noticed."]);
  assert.equal(VOICE_ROOM_REST.left, 30000);
  assert.ok(css.includes(`${DESK} #app.vr-open .voice-panel[data-dock="left"] .vp-where { overflow: hidden; text-overflow: clip; white-space: nowrap; }`));
});

test("while you are out everyone is a card, yours is marked, and nobody's picture plays", () => {
  const inside = voiceRoomPeople({ userIds: ["a", "b", "c"], names: ["ash", "bo", "cy"], muted: [false, true, false], deafened: [], speaking: [], shares: [{ user: "a" }], cameras: ["c"], meId: "me" });
  const out = voiceRoomAway(inside, { id: "me", name: "Me", at: 1, shared: false });
  assert.deepEqual(out.map(person => person.id), ["a", "me", "b", "c"]);
  assert.deepEqual(out.map(person => person.live), [true, false, false, true]);
  assert.ok(out.every(person => !person.camera));
  assert.deepEqual(out[1], { id: "me", name: "Me", me: true, away: true, live: false, camera: false, state: "left" });
  assert.equal(out[2].state, "muted");
  assert.equal(voiceRoomAway(inside, { id: "me", name: "Me", at: 1, shared: true })[1].state, "");
  assert.deepEqual(voiceRoomAway(inside, { id: "me", name: "Me", at: -1 }).map(person => person.id), ["a", "b", "c", "me"]);
  assert.deepEqual(voiceRoomAway(inside, { id: "me", name: "Me", at: 9 }).map(person => person.id), ["a", "b", "c", "me"]);
  assert.deepEqual(voiceRoomAway([...inside, { id: "me", name: "Me", me: true }], { id: "me", name: "Me", at: 0 }).map(person => person.id), ["me", "a", "b", "c"]);
  assert.match(app, /const word = person\.state === "left" \? "left the room" : quiet;\n    card\.classList\.toggle\("away", Boolean\(person\.away\)\);/);
  assert.match(app, /if \(card\.classList\.contains\("away"\)\) return;/);
  assert.ok(css.includes(`${DESK} .vr-card.away { opacity: .5; cursor: default; }`));
  assert.match(app, /\$\("vr-invite"\)\.classList\.toggle\("hidden", !server\.can_invite \|\| Boolean\(left\)\);/);
  assert.match(app, /\$\("vr-activities"\)\.classList\.toggle\("hidden", Boolean\(left\) \|\| /);
});

test("your stream's tile stays as dark glass when you leave or stop, in the place it had", () => {
  assert.deepEqual(VOICE_ROOM_ENDED.left, ["You left the room.", "Your screen stopped with you."]);
  assert.deepEqual(VOICE_ROOM_ENDED.stopped, ["You stopped sharing.", "Share Screen brings it back."]);
  assert.equal(VOICE_ROOM_REST.stopped, 4000);
  const tiles = [{ key: "screen:a", kind: "screen" }, { key: "camera:b", kind: "camera" }];
  assert.deepEqual(voiceRoomGhostTiles(tiles, "screen:me").map(tile => tile.key), ["screen:a", "screen:me", "camera:b"]);
  assert.equal(voiceRoomGhostTiles(tiles, "screen:me")[1].kind, "screen");
  assert.equal(voiceRoomGhostTiles(tiles, "screen:a"), tiles);
  assert.equal(voiceRoomGhostTiles(tiles, ""), tiles);
  assert.deepEqual(voiceRoomGhostTiles([], "screen:me").map(tile => tile.key), ["screen:me"]);
  const laid = voiceRoomLayout({ width: 1032, height: 600, items: voiceRoomItems(voiceRoomGhostTiles([], "screen:me"), [{ id: "a" }, { id: "me" }]) });
  assert.equal(laid.mode, "quarters");
  assert.equal(laid.placed[0].key, "screen:me");
  assert.match(app, /if \(wanted\.has\(tile\.dataset\.key\)\) continue;\n    keepVoiceRoomGhost\(tile\);\n    tilePainter\(tile\)\.clear\(\);/);
  assert.match(app, /if \(ghost && tiles\.some\(t => t\.key === ghost\.key\)\) dropVoiceRoomGhost\(\);/);
  assert.match(app, /\}, VOICE_ROOM_REST\.stopped\);/);
  for (const part of [".vr-ghost {", ".vr-ghost .vr-still {", ".vr-ended {", ".vr-ended b { font: 700 17px/1.2 var(--font-display); letter-spacing: -.01em; }"]) assert.ok(css.includes(`${DESK} ${part}`), part);
});

test("the room waits thirty seconds with a Rejoin key where hang up was, and its keys rest", () => {
  assert.match(app, /timer: setTimeout\(\(\) => closeVoiceRoom\(\), VOICE_ROOM_REST\.left\),/);
  assert.match(app, /await invoke\("set_voice_channel", \{ serverId: state\.voiceServer\.id, channelId: null \}\);\n    restVoiceRoom\(room, shared\);\n    state\.voiceChannelId = null;/);
  assert.match(app, /panel\.classList\.toggle\("hidden", !\(state\.voiceChannelId && vs\) && !voiceRoomLeft\(\)\);/);
  assert.match(app, /return left && voiceRoom\.open && studioDesk\(\) && state\.view === "server" && state\.currentServer && state\.currentServer\.id === left\.server \? left : null;/);
  assert.match(app, /if \(voiceRoom\.left && callChannelHere\(\)\) dropVoiceRoomLeft\(\);/);
  assert.match(app, /voiceRoom\.open = false;\n  dropVoiceRoomLeft\(\);\n  dropVoiceRoomGhost\(\);/);
  assert.match(app, /for \(const id of \["mute-btn", "deafen-btn", "vp-camera", "share-btn", "vp-soundboard", "vp-hand-key"\]\) \{\n    const key = document\.getElementById\(id\);\n    if \(key\) key\.disabled = resting;/);
  assert.match(app, /key = el\("button", "key primary vp-rejoin hidden"\);/);
  assert.match(app, /await toggleVoice\(channel, true\);/);
  assert.match(app, /back \? `Back in \$\{channelTextName\(channel\.name\)\}\.` :/);
  assert.ok(css.includes(`${DESK} #app.vr-open .voice-panel .vp-rejoin {`));
  assert.ok(css.includes(`${DESK} #app.vr-open .voice-panel[data-dock="left"] .vp-hangup,`));
  assert.ok(css.includes(`${DESK} #app.vr-open .voice-panel[data-dock="left"] .vp-meter i { background: var(--text-faint); scale: 1 .08; animation: none; }`));
  assert.ok(css.includes(`${DESK} #app.vr-open .vp-activities.vp-share:not(.active)::after {`));
});

test("an expanded stream carries the tile's chips and keys, and the old stage's own controls are gone", () => {
  const dress = app.slice(app.indexOf("function dressVoiceRoomStage()"), app.indexOf("function stripVoiceRoomStage()"));
  const listed = dress.slice(dress.indexOf("keys.append("));
  const order = ["draw,", "vr-point", "vr-wipe", "volume,", "vr-big", "vr-x"].map(part => listed.indexOf(part));
  assert.ok(order.every((at, i) => at >= 0 && (i === 0 || at > order[i - 1])), String(order));
  assert.match(dress, /voiceRoomTileKey\("sq vr-big", "Back to the grid", "minimize-2", "", \(\) => backToCallGrid\(\)\)/);
  assert.match(dress, /voiceRoomTileKey\("sq vr-x", "Stop watching", "x", "", \(\) => closeStageButton\(\)\)/);
  assert.match(dress, /el\("span", null, "The room can draw on this"\)/);
  assert.match(app, /if \(!voiceRoom\.open \|\| state\.stageMode !== "watch" \|\| !state\.watchingUser\) return stripVoiceRoomStage\(\);/);
  assert.match(app, /for \(const part of \["\.vr-draw", "\.vr-point", "\.vr-wipe", "\.vr-allowed"\]\) view\.querySelector\(part\)\.classList\.toggle\("hidden", !allowed\);/);
  assert.match(app, /key\.classList\.toggle\("is-on", allowed && ink\.tool === tool\);/);
  assert.match(app, /\$\("vr-cards"\)\.replaceChildren\(\);\n  stripVoiceRoomStage\(\);/);
  assert.ok(css.includes(`${DESK} .vr-grid .stage-view:has(> .vr-big-part) > .stage-top,`));
  assert.ok(css.includes(`${DESK} .vr-grid .stage-view:has(> .vr-big-part) > .stage-controls { display: none; }`));
  assert.ok(css.includes(`${DESK} .vr-grid[data-mode="expanded"] .cam-tile .vr-keys,`));
  assert.ok(css.includes(`${DESK} .vr-grid .stage-view:has(> .vr-big-part) { overflow: visible; }`));
});

test("the stream's size and frame rate show on the Live chip, a double click or Esc goes back to the grid", () => {
  assert.match(app, /const live = el\("span", "live-badge vr-live", "Live"\);\n  live\.tabIndex = 0;/);
  assert.match(app, /const text = \$\("stage-stats"\)\.textContent;\n  if \(readout\.textContent !== text\) readout\.textContent = text;\n  readout\.classList\.toggle\("hidden", !text\);/);
  assert.match(app, /join\(" \\u00b7 "\);\n  paintVoiceRoomReadout\(\);/);
  assert.ok(css.includes(`${DESK} .vr-big-who .vr-live:hover ~ .vr-readout,`));
  assert.ok(css.includes(`${DESK} .vr-big-who .vr-live:focus-visible ~ .vr-readout { display: flex; }`));
  assert.match(app, /\$\("stage-view"\)\.ondblclick = \(\) => \{\n    if \(!voiceRoom\.open\) return toggleStageFullscreen\(\);\n    if \(!ink\.tool && state\.stageMode === "watch"\) backToCallGrid\(\);/);
  assert.match(app, /if \(pop && !pop\.classList\.contains\("hidden"\)\) return closeVoiceRoomPop\(\);\n    if \(voiceRoom\.pen\.tool\) return setVoiceRoomTool\(null, null\);\n    if \(state\.stageMode !== "watch"\) return;\n    if \(ink\.tool\) return setVoiceRoomStageTool\(ink\.tool\);\n    backToCallGrid\(\);/);
  const laid = voiceRoomLayout({ width: 1032, height: 600, items: [...streams(2), ...cameras(1), ...people(3)], expanded: "screen:1" });
  assert.equal(laid.mode, "expanded");
  assert.deepEqual(laid.placed.map(spot => spot.key), ["screen:1", "screen:0", "camera:0", "person:0", "person:1", "person:2"]);
  assert.deepEqual(laid.placed.map(spot => spot.form), ["big", "mini", "mini", "mini", "mini", "mini"]);
});
