import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  PHONE_TABS, HORIZON, HORIZON_SPRINGS, HANG_UP_FUSE_MS, horizonY, horizonPath, horizonLift, springStep,
  phoneTabAfter, hangUpTap, badgeText, roomLine, liveSummary, liveRooms, liveStreams, stackPeople,
  SWIPE, QUICK_REACTIONS, SHEET_ROWS, swipeAxis, replyPull, replyCommits, edgeBackStarts, edgeBackCommits,
  rowPull, rowStaysOpen, sheetPull, sheetCloses, BAR_EXTRAS, sheetRow, quickReactions, chatSubtitle,
  ROOM_KEYS, roomTiles, duoTiles, roomHeading, roomStreams, signalWord, roomKeyState, roomSignature, streamBubbles, watchPlace,
} = require("../scripts/phone-touch.js");
const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("styles", "phone-touch.css");
const glue = read("scripts", "studio-phone.js");
const html = read("index.html");

test("the line is flat away from your tab and rises nine pixels under it", () => {
  assert.equal(horizonY(40, 300, HORIZON.swell, null, 0).toFixed(2), "30.00");
  assert.equal(horizonY(300, 300, HORIZON.swell, null, 0), 21);
  assert.equal(horizonLift(300, 300), -9);
  assert.ok(Math.abs(horizonLift(40, 300)) < 0.001);
});

test("a new message lifts the line a little under its own tab", () => {
  assert.ok(horizonY(120, 300, HORIZON.swell, 120, 5) < horizonY(120, 300, HORIZON.swell, null, 0));
  assert.equal(horizonY(120, 300, HORIZON.swell, 120, 0), horizonY(120, 300, HORIZON.swell, null, 0));
});

test("the line is one path that runs past both edges of the bar", () => {
  const d = horizonPath(390, 60);
  assert.match(d, /^M-6 /);
  assert.ok(d.includes("L393 ") || d.includes("L396 "));
  assert.equal(d.split("L").length - 1, Math.floor((390 + 12) / HORIZON.step));
});

test("a spring reaches its target, overshoots at most a little, and then rests", () => {
  let spring = { ...HORIZON_SPRINGS.slide, x: 0, v: 0, target: 100 };
  let peak = 0;
  let frames = 0;
  while (!spring.settled && frames < 600) {
    spring = springStep(spring, 1 / 60);
    peak = Math.max(peak, spring.x);
    frames++;
  }
  assert.equal(spring.settled, true);
  assert.equal(spring.x, 100);
  assert.ok(peak <= 106, `peak ${peak}`);
  assert.ok(frames < 90, `frames ${frames}`);
});

test("a long pause between frames cannot throw the spring", () => {
  const spring = springStep({ ...HORIZON_SPRINGS.hop, x: 9, v: 190, target: 9 }, 5);
  assert.ok(Number.isFinite(spring.x) && Math.abs(spring.x - 9) < 12);
});

test("there are four places, and opening a conversation remembers which one it belongs to", () => {
  assert.deepEqual(PHONE_TABS, ["home", "messages", "live", "you"]);
  assert.equal(phoneTabAfter("home", { pane: "list", view: "server" }), "home");
  assert.equal(phoneTabAfter("home", { pane: "list", view: "dm" }), "messages");
  assert.equal(phoneTabAfter("messages", { pane: "list", view: "server" }), "home");
  assert.equal(phoneTabAfter("live", { pane: "list", view: "server" }), "live");
  assert.equal(phoneTabAfter("you", { pane: "list", view: "dm" }), "you");
  assert.equal(phoneTabAfter("live", { pane: "chat", view: "server" }), "home");
  assert.equal(phoneTabAfter("you", { pane: "chat", view: "dm" }), "messages");
  assert.equal(phoneTabAfter("nonsense", { pane: "list", view: "server" }), "home");
});

test("hanging up asks twice, with three seconds to change your mind", () => {
  assert.equal(hangUpTap(false), "arm");
  assert.equal(hangUpTap(true), "leave");
  assert.equal(HANG_UP_FUSE_MS, 3000);
});

test("badges and room lines read plainly", () => {
  assert.equal(badgeText(0), "");
  assert.equal(badgeText(3), "3");
  assert.equal(badgeText(250), "99+");
  assert.equal(roomLine(0, false), "");
  assert.equal(roomLine(1, false), "1 person in the room");
  assert.equal(roomLine(4, false), "4 in the room");
  assert.equal(roomLine(4, true), "You're connected");
  assert.equal(liveSummary(1, 2), "1 stream, right now");
  assert.equal(liveSummary(3, 3), "3 streams, right now");
  assert.equal(liveSummary(0, 1), "1 room has people in it");
  assert.equal(liveSummary(0, 0), "");
});

const servers = [
  {
    id: "s1", name: "Pixel Pioneers",
    channels: [
      { id: "c1", name: "general", kind: "text" },
      { id: "v1", name: "Lounge", kind: "voice", voice_users: ["quill", "sprocket"], voice_user_ids: ["m3", "m6"] },
      { id: "v2", name: "Quiet Room", kind: "voice", voice_users: [] },
    ],
    screen_shares: [{ user: "m6", name: "sprocket", channel: "v1" }],
  },
  { id: "s2", name: "Kart Club", channels: [{ id: "v3", name: "Pit Lane", kind: "voice", voice_users: ["juno"], voice_user_ids: ["m2"] }], screen_shares: [{ user: "m2", name: "juno" }] },
  { id: "g1", name: "group", group: true, channels: [{ id: "v4", name: "call", kind: "voice", voice_users: ["a"], voice_user_ids: ["a"] }] },
];

test("the Live page lists rooms with people in them, and who is streaming in each", () => {
  const rooms = liveRooms(servers, { userId: "me", channelId: "v1" });
  assert.deepEqual(rooms.map(room => room.channelName), ["Lounge", "Pit Lane"]);
  assert.equal(rooms[0].here, true);
  assert.equal(rooms[1].here, false);
  assert.deepEqual(rooms[0].streams, [{ userId: "m6", name: "sprocket", mine: false }]);
  assert.deepEqual(rooms[1].streams.map(stream => stream.userId), ["m2"]);
  const streams = liveStreams(rooms);
  assert.equal(streams.length, 2);
  assert.equal(streams[0].room.serverName, "Pixel Pioneers");
  assert.deepEqual(liveRooms([], {}), []);
});

test("a row of faces shows four people and counts the rest", () => {
  const people = ["a", "b", "c", "d", "e", "f"].map(name => ({ id: name, name }));
  assert.deepEqual(stackPeople(people).shown.map(p => p.name), ["a", "b", "c", "d"]);
  assert.equal(stackPeople(people).more, 2);
  assert.equal(stackPeople(people.slice(0, 2)).more, 0);
});

test("every phone rule waits for the new look and a slim window", () => {
  const selectors = css.replace(/@keyframes[^{]+\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g, "").match(/(?:^|\})\s*([^{}@]+)\{/g) || [];
  assert.ok(selectors.length > 150);
  for (const raw of selectors) {
    for (const selector of raw.replace(/^\}/, "").replace(/\{$/, "").replace(/\([^()]*\)/g, "").split(",")) {
      if (selector.trim().startsWith('html[data-look="studio"]:not ')) continue;
      assert.match(selector.trim(), /^html\[data-look="studio"\]\[data-fit="slim"\]/, selector.trim());
    }
  }
});

test("the phone-only parts stay hidden in every wider window", () => {
  const rule = css.match(/^html\[data-look="studio"\]:not\(\[data-fit="slim"\]\) :is\(([^)]+)\) \{ display: none !important; \}$/m);
  assert.ok(rule);
  const hidden = rule[1].split(", ");
  for (const built of glue.matchAll(/(?:el|studioNode)\("[a-z]+", "((?:pt-|hzn)[a-z-]*)/g)) assert.ok(hidden.includes(`.${built[1]}`), built[1]);
  assert.ok(hidden.includes(".pt-room-chip"));
});

test("the phone stylesheet carries no colours of its own", () => {
  assert.doesNotMatch(css.replace(/#000\b/g, ""), /#[0-9a-fA-F]{3,8}\b/);
  assert.doesNotMatch(css, /\brgba?\(\s*\d/);
});

test("the phone layout builds nothing unless the new look is on", () => {
  assert.match(glue, /function wireStudioPhone\(\) \{\n  if \(!studioLook\(\)/);
  assert.match(html, /<link rel="stylesheet" id="studio-phone" href="styles\/phone-touch\.css" media="not all" \/>/);
  assert.ok(html.indexOf('scripts/phone.js"') < html.indexOf('scripts/phone-touch.js"'));
  assert.ok(html.indexOf('scripts/app.js"') < html.indexOf('scripts/studio-phone.js"'));
});

test("nothing on the phone offers to share an iPhone screen, and the bar names no small targets", () => {
  assert.doesNotMatch(glue, /share screen/i);
  for (const size of css.matchAll(/\.hzn-voice \.key \{[^}]*width: (\d+)px;[^}]*height: (\d+)px/g)) {
    assert.ok(Number(size[1]) >= 44 && Number(size[2]) >= 44);
  }
});

test("a swipe is sideways only once it clearly is, so scrolling the chat never drags a message", () => {
  assert.equal(swipeAxis(6, 4), null);
  assert.equal(swipeAxis(30, 8), "x");
  assert.equal(swipeAxis(-30, 8), "x");
  assert.equal(swipeAxis(14, 14), "y");
  assert.equal(swipeAxis(4, 40), "y");
});

test("pulling a message right follows the finger to 84 px and replies past 56", () => {
  assert.equal(SWIPE.replyAt, 56);
  assert.equal(SWIPE.replyMax, 84);
  assert.equal(replyPull(-20), 0);
  assert.equal(replyPull(NaN), 0);
  assert.equal(replyPull(30), 30);
  assert.equal(replyPull(SWIPE.replyAt), SWIPE.replyAt);
  assert.equal(replyPull(70), 70);
  assert.equal(replyPull(SWIPE.replyMax), SWIPE.replyMax);
  assert.equal(replyPull(900), SWIPE.replyMax);
  assert.equal(replyCommits(SWIPE.replyAt - 1), false);
  assert.equal(replyCommits(SWIPE.replyAt), true);
});

test("a sheet follows the finger down and closes past the mark, never up", () => {
  assert.equal(sheetPull(-30), 0);
  assert.equal(sheetPull(NaN), 0);
  assert.equal(sheetPull(48), 48);
  assert.equal(sheetCloses(SWIPE.sheetAt - 1), false);
  assert.equal(sheetCloses(SWIPE.sheetAt), true);
  assert.equal(sheetCloses(-200), false);
  const glue = read("scripts", "studio-phone.js");
  for (const sheet of ["#pt-hold", ".ctx-menu", ".composer-menu", ".audio-pop", ".soundboard-pop"]) assert.ok(glue.includes(sheet), sheet);
});

test("the phone chat bar is plus, field and Send, with the rest behind plus", () => {
  const css = read("styles", "phone-touch.css");
  assert.match(css, /html\[data-look="studio"\]\[data-fit="slim"\] \.composer :is\(#text-style-btn, #gif-btn, #emoji-btn\) \{ display: none; \}/);
  const page = read("index.html");
  for (const extra of BAR_EXTRAS) {
    assert.ok(page.includes(`id="${extra.press}"`), extra.press);
    assert.ok(extra.label && extra.icon && extra.id);
  }
  assert.ok(page.includes('id="cm-gif"'));
  assert.ok(page.includes('id="reply-bar"') && page.includes('id="reply-bar-cancel"'));
});

test("starting a reply moves nothing: the chip sits in the gap above the field", () => {
  const css = read("styles", "phone-touch.css");
  const at = css.indexOf('html[data-look="studio"][data-fit="slim"] .app[data-pane="chat"] .reply-bar {');
  assert.ok(at > 0);
  const body = css.slice(at, css.indexOf("}", at));
  assert.match(body, /height: 30px;/);
  assert.match(body, /margin: -34px 0 4px 68px;/);
  assert.match(css, /\.app\[data-pane="chat"\] \.messages \{ margin-bottom: 38px; \}/);
});

test("mentions are never red in the phone layout, and times read like the guide", () => {
  const css = read("styles", "phone-touch.css");
  assert.match(css, /html\[data-look="studio"\]\[data-fit="slim"\] \.mention \{[^}]*var\(--accent\)/);
  assert.match(css, /\.mention\.me \{ background: var\(--accent-2\); color: var\(--accent-2-ink\); \}/);
  assert.match(read("scripts", "app.js"), /hour: studioLook\(\) \? "numeric" : "2-digit"/);
});

test("going back starts at the left edge and needs a real pull", () => {
  assert.equal(edgeBackStarts(0), true);
  assert.equal(edgeBackStarts(SWIPE.edge), true);
  assert.equal(edgeBackStarts(SWIPE.edge + 1), false);
  assert.equal(edgeBackCommits(SWIPE.backAt - 1), false);
  assert.equal(edgeBackCommits(SWIPE.backAt), true);
});

test("a row opens far enough for two full-size actions and snaps open or shut", () => {
  assert.ok(SWIPE.rowOpen >= 2 * 44 + 16);
  assert.equal(rowPull(-40, false), 0);
  assert.equal(rowPull(50, false), 50);
  assert.ok(rowPull(400, false) <= SWIPE.rowOpen + 18);
  assert.equal(rowPull(-SWIPE.rowOpen, true), 0);
  assert.equal(rowPull(0, true), SWIPE.rowOpen);
  assert.equal(rowStaysOpen(SWIPE.rowAt - 1, false), false);
  assert.equal(rowStaysOpen(SWIPE.rowAt, false), true);
  assert.equal(rowStaysOpen(SWIPE.rowOpen - 10, true), true);
  assert.equal(rowStaysOpen(SWIPE.rowOpen - SWIPE.rowAt, true), false);
});

test("the sheet names every message action in plain words and marks the ones that delete", () => {
  for (const title of ["Reply", "Open Thread", "Pin", "Unpin", "Edit", "Hide for Me", "Save", "Unsave", "Remind Me", "More", "Report Message", "Delete", "Delete (Moderator)"]) {
    assert.ok(SHEET_ROWS[title].label, title);
    assert.ok(SHEET_ROWS[title].icon, title);
  }
  assert.equal(sheetRow("Delete").danger, true);
  assert.equal(sheetRow("Delete (Moderator)").danger, true);
  assert.equal(sheetRow("Reply").danger, undefined);
  assert.deepEqual(sheetRow("Translate Message"), { label: "Translate message", icon: null });
  assert.deepEqual(sheetRow(""), { label: "", icon: null });
});

test("the reaction row is your most used emoji first, filled up from a fixed five, never repeated", () => {
  assert.deepEqual(quickReactions([]), QUICK_REACTIONS);
  assert.deepEqual(quickReactions(null), QUICK_REACTIONS);
  const mine = quickReactions(["\u{1F3C1}", QUICK_REACTIONS[0], ":custom:"]);
  assert.equal(mine.length, QUICK_REACTIONS.length);
  assert.equal(mine[0], "\u{1F3C1}");
  assert.equal(mine[1], QUICK_REACTIONS[0]);
  assert.equal(new Set(mine).size, mine.length);
  assert.ok(!mine.includes(":custom:"));
  assert.equal(quickReactions([], 3).length, 3);
});

test("the line under the chat title says where you are", () => {
  assert.equal(chatSubtitle({ view: "server", serverName: "Pixel Pioneers" }), "Pixel Pioneers");
  assert.equal(chatSubtitle({ view: "server", serverName: "Friends", group: true }), "Group chat");
  assert.equal(chatSubtitle({ view: "dm", presence: "Online" }), "Online");
  assert.equal(chatSubtitle({ view: "dm", presence: "" }), "Direct message");
  assert.equal(chatSubtitle({ view: "server" }), "");
});

test("every sheet and long-press menu on the phone has full-size rows", () => {
  const height = selector => {
    const rule = css.match(new RegExp(`^html\\[data-look="studio"\\]\\[data-fit="slim"\\] ${selector} \\{[^}]*min-height: (\\d+)px`, "m"));
    assert.ok(rule, selector);
    return Number(rule[1]);
  };
  assert.ok(height("\\.arow") >= 44);
  assert.ok(height(":is\\(\\.ctx-menu, \\.composer-menu\\) button") >= 44);
  assert.match(css, /\.pt-hold \.rx-key \{ width: (4[4-9]|[5-9]\d)px; height: (4[4-9]|[5-9]\d)px;/);
  assert.match(css, /\.pt-acts \.act \{[^}]*min-height: (4[4-9]|[5-9]\d)px/);
});

test("the hover toolbar on a message never shows on the phone, the sheet reads it instead", () => {
  assert.match(css, /^html\[data-look="studio"\]\[data-fit="slim"\] \.msg-actions \{ display: none !important; \}$/m);
  assert.match(glue, /messageOfActions\.get\(/);
  assert.match(read("scripts", "app.js"), /messageOfActions\.set\(bar, message\);/);
});

test("swipe to reply follows the guide: the message slides on a rail and the reply key lights past the mark", () => {
  const rule = name => {
    const found = css.match(new RegExp(`^html\\[data-look="studio"\\]\\[data-fit="slim"\\] ${name} \\{([^}]*)\\}`, "m"));
    assert.ok(found, name);
    return found[1];
  };
  assert.match(rule("\\.pt-reply-cue\\.on"), /position: absolute;[\s\S]*background: var\(--input-bg\)/);
  assert.match(rule("\\.pt-reply-cue \\.rk"), /background: var\(--swipe-neutral\)/);
  assert.match(css, /\.pt-reply-cue\.ready \.rk \{ background: var\(--swipe-done\)/);
  assert.match(rule("\\.msg\\.pt-railed\\.pt-lifted"), /background: var\(--bg-raised\);\s*box-shadow: var\(--pillow-card\)/);
  assert.match(rule("\\.msg\\.pt-railed"), /border-radius: 22px/);
  assert.doesNotMatch(css, /\.pt-reply-cue[^{]*\{[^}]*position: fixed/);
  assert.match(glue, /node\.after\(cue\);/);
  assert.equal(SWIPE.replyAt, 56);
});

const lounge = {
  id: "v1", name: "Lounge", kind: "voice",
  voice_users: ["quill", "ferris", "commander"], voice_user_ids: ["m3", "m1", "me"],
  voice_muted: [false, true, false], voice_deafened: [false, false, false],
};
const pioneers = { id: "s1", name: "Pixel Pioneers", channels: [lounge], screen_shares: [{ user: "m1", name: "ferris", channel: "v1" }, { user: "me", name: "commander", channel: "v1" }] };

test("the room has five keys on the line with the microphone in the middle and leaving at the end", () => {
  assert.deepEqual(ROOM_KEYS.map(key => key.id), ["sounds", "camera", "mic", "deafen", "leave"]);
  assert.equal(ROOM_KEYS[Math.floor(ROOM_KEYS.length / 2)].id, "mic");
  assert.match(glue, /key\.id === "leave" \? "tap danger hang"/);
});

test("the room shows everyone in it, you by the word You, and who is muted", () => {
  const tiles = roomTiles(lounge, { userId: "me", muted: true, deafened: false });
  assert.deepEqual(tiles.map(tile => tile.label), ["quill", "ferris", "You"]);
  assert.equal(tiles[2].name, "commander");
  assert.equal(tiles[2].you, true);
  assert.equal(tiles[2].muted, true);
  assert.equal(tiles[1].muted, true);
  assert.equal(tiles[0].muted, false);
  assert.deepEqual(roomTiles(null), []);
  assert.deepEqual(duoTiles("ferris", { userId: "me", name: "commander", muted: true }).map(tile => [tile.label, tile.muted]), [["ferris", false], ["You", true]]);
});

test("the room heading counts who is here and a call says it is a call", () => {
  assert.equal(roomHeading({ serverName: "Pixel Pioneers", count: 5 }), "Pixel Pioneers \u00B7 5 here");
  assert.equal(roomHeading({ serverName: "Pixel Pioneers", count: 1 }), "Pixel Pioneers \u00B7 1 here");
  assert.equal(roomHeading({ count: 2 }), "2 here");
  assert.equal(roomHeading({ serverName: "ferris", count: 2, call: true }), "Voice call");
  assert.equal(watchPlace({ channelName: "Lounge", name: "Pixel Pioneers" }), "Lounge \u00B7 Pixel Pioneers");
  assert.equal(watchPlace({ name: "ferris", call: true }), "Call with ferris");
  assert.equal(watchPlace(null), "");
});

test("the room lists other people's streams, never your own, and knows the one you are watching", () => {
  const streams = roomStreams(pioneers, "v1", { userId: "me", watching: "m1" });
  assert.deepEqual(streams, [{ user: "m1", name: "ferris", title: "ferris's screen", watching: true }]);
  assert.equal(roomStreams(pioneers, "v1", { userId: "me" })[0].watching, false);
  assert.deepEqual(roomStreams(pioneers, "nope", { userId: "me" }), []);
  assert.deepEqual(roomStreams(null, "v1"), []);
});

test("muted and deafened are a state of the key, and the camera says when it is off", () => {
  assert.deepEqual(roomKeyState("mic", { muted: true }), { down: true, icon: "mic-off", label: "Unmute" });
  assert.deepEqual(roomKeyState("mic", {}), { down: false, icon: "mic", label: "Mute" });
  assert.equal(roomKeyState("deafen", { deafened: true }).label, "Undeafen");
  assert.equal(roomKeyState("camera", {}).off, true);
  assert.equal(roomKeyState("camera", {}).icon, "video-off");
  assert.equal(roomKeyState("camera", { camera: true }).off, false);
  assert.equal(roomKeyState("sounds", { sounds: true }).down, true);
  assert.equal(signalWord("good"), "Good");
  assert.equal(signalWord("ok"), "Fair");
  assert.equal(signalWord("bad"), "Poor");
  assert.equal(signalWord("unknown"), "");
});

test("the room only redraws when who is in it, or what they share, changes", () => {
  const tiles = roomTiles(lounge, { userId: "me" });
  const streams = roomStreams(pioneers, "v1", { userId: "me" });
  assert.equal(roomSignature(tiles, streams, "x"), roomSignature(roomTiles(lounge, { userId: "me" }), streams, "x"));
  assert.notEqual(roomSignature(tiles, streams, "x"), roomSignature(roomTiles(lounge, { userId: "me", muted: true }), streams, "x"));
  assert.notEqual(roomSignature(tiles, streams, "x"), roomSignature(tiles, [], "x"));
  assert.notEqual(roomSignature(tiles, streams, "x"), roomSignature(tiles, streams, "y"));
});

test("the chat over a stream is the last three things said, on one line each", () => {
  const rows = [
    { id: "1", author: "juno", text: "first" },
    { id: "2", author: "juno", text: "   " },
    { id: "3", author: "quill", text: "second\n  line" },
    { id: "4", author: "", text: "nobody" },
    { id: "5", author: "ferris", text: "x".repeat(200) },
    { id: "6", author: "commander", color: "red", text: "last" },
  ];
  const said = streamBubbles(rows);
  assert.deepEqual(said.map(row => row.id), ["3", "5", "6"]);
  assert.equal(said[0].text, "second line");
  assert.equal(said[1].text.length, 72);
  assert.ok(said[1].text.endsWith("\u2026"));
  assert.equal(said[2].color, "red");
  assert.deepEqual(streamBubbles(null), []);
  assert.equal(streamBubbles(rows, 1).length, 1);
});

test("nothing the phone scripts name at the top level is already named by another script", () => {
  const names = text => new Set([...text.matchAll(/^(?:async function|function|const|let|var|class) ([A-Za-z_$][\w$]*)/gm)].map(found => found[1]));
  const mine = new Set([...names(read("scripts", "phone-touch.js")), ...names(glue)]);
  const overlap = [...names(read("scripts", "phone-touch.js"))].filter(name => names(glue).has(name));
  assert.deepEqual(overlap, []);
  const folder = new URL("../scripts/", import.meta.url);
  for (const file of readdirSync(folder)) {
    if (!file.endsWith(".js") || file === "phone-touch.js" || file === "studio-phone.js") continue;
    const theirs = names(read("scripts", file));
    for (const name of mine) assert.ok(!theirs.has(name), `${name} is also in ${file}`);
  }
});

test("over a stream every key is full size and the stream fills the screen only in the chat pane", () => {
  assert.match(css, /\.app\.pt-stream-full\[data-pane="chat"\] \.stage:not\(\.hidden\) \{[^}]*position: fixed;[^}]*inset: 0;/);
  assert.match(css, /\.pt-watch \.ctl \.key\.sq \{ width: (4[4-9]|[5-9]\d)px; height: (4[4-9]|[5-9]\d)px; \}/);
  assert.match(css, /\.pt-watch \.wbar \.hang \{[^}]*width: (4[4-9]|[5-9]\d)px; height: (4[4-9]|[5-9]\d)px;/);
  assert.match(css, /\.stage-btn \{ flex: none; width: (4[4-9]|[5-9]\d)px; height: (4[4-9]|[5-9]\d)px; \}/);
  assert.match(glue, /classList\.toggle\("theme-dark", full\)/);
});
