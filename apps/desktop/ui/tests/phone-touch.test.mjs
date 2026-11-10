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
  ROOM_KEYS, roomTiles, duoTiles, roomHeading, roomStreams, signalWord, roomKeyState, roomSignature, streamBubbles, watchPlace, roomFaces, TILE_NAME, tileNameWidth,
  WAIT_CARDS, waitCards, waitMoreWords, listAge, listLabel, unreadChats,
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
  assert.equal(SWIPE.slop, 6);
  assert.equal(swipeAxis(4, 3), null);
  assert.equal(swipeAxis(6, 2), "x");
  assert.equal(swipeAxis(6, 5), "y");
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

test("the reply chip arrives a beat after the finger lets go, as the guide plays it", () => {
  assert.equal(SWIPE.chipAfterMs, 110);
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
  assert.match(body, /margin: -33px 0 3px 70px;/);
  assert.match(css, /\.app\[data-pane="chat"\] \.messages \{\s+margin: 6px 10px 51px;/);
});

test("the chat screen keeps the design guide's measures", () => {
  const css = read("styles", "phone-touch.css");
  const rule = selector => {
    const at = css.indexOf(`html[data-look="studio"][data-fit="slim"] ${selector} {`);
    assert.ok(at >= 0, selector);
    return css.slice(at, css.indexOf("}", at));
  };
  assert.match(css, /--hz-safe: max\(env\(safe-area-inset-bottom\), 8px\);/);
  assert.match(rule('.app:is([data-pane="chat"], [data-pane="people"])'), /padding: max\(env\(safe-area-inset-top\), 14px\) 0 0;/);
  assert.match(rule('.app:is([data-pane="chat"], [data-pane="people"]) .chat-header'), /height: 60px;[\s\S]*padding: 0 14px 0 6px;/);
  assert.match(rule('.app:is([data-pane="chat"], [data-pane="people"]) #chat-hash'), /font: 300 22px\/22px var\(--font-display\);[\s\S]*color: var\(--link\);/);
  assert.match(rule('.app[data-pane="chat"] > .main.pt-pane::before'), /left: 10px;\s+right: 10px;[\s\S]*border-radius: var\(--radius-xl\);/);
  assert.match(rule('.app[data-pane="chat"] .msg'), /grid-template-columns: 40px minmax\(0, 1fr\);\s+gap: 0 12px;\s+margin: 0;\s+padding: 8px 8px 10px;/);
  assert.match(rule('.app[data-pane="chat"] .msg-author'), /font: 600 15\.5px\/1 var\(--font\);/);
  assert.match(rule('.app[data-pane="chat"] .msg-time'), /font: 400 13px\/1 var\(--font\);/);
  assert.match(rule('.msg .reaction'), /height: 32px;[\s\S]*padding: 0 13px 0 10px;[\s\S]*background: var\(--pillow-top\), var\(--bg-active\);/);
  assert.match(rule('.msg .reaction.mine'), /background: var\(--pillow-top\), var\(--accent\); color: var\(--accent-ink\);/);
  assert.match(rule('.app[data-pane="chat"] .msg.pt-railed'), /margin-inline: -4px;\s+padding-inline: 12px;\s+border-radius: 22px;/);
  assert.match(rule('.app[data-pane="chat"] .composer'), /margin: 0 16px calc\(var\(--hz-line\) \+ 7px\);/);
  assert.match(rule('.app[data-pane="chat"] .typing-strip'), /left: 22px;\s+bottom: calc\(var\(--hz-line\) - 30px\);/);
  assert.match(rule('.app[data-pane="chat"] .msg .chat-image'), /max-width: 100%; max-height: 300px;[^}]*border-radius: 16px;/);
});

test("the typing line shows at most three faces, and the reply key changes colour at once", () => {
  const js = read("scripts", "studio-phone.js");
  assert.match(js, /const STUDIO_TYPER_FACES = 3;/);
  assert.match(js, /\.slice\(0, STUDIO_TYPER_FACES\)/);
  const css = read("styles", "phone-touch.css");
  const at = css.indexOf('html[data-look="studio"][data-fit="slim"] .pt-reply-cue .rk {');
  const body = css.slice(at, css.indexOf("}", at));
  assert.match(body, /transition: scale 180ms var\(--ease-spring\);/);
  assert.doesNotMatch(body, /background-color var/);
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
  assert.match(rule('\\.app\\[data-pane="chat"\\] \\.msg\\.pt-railed\\.pt-lifted'), /background: var\(--bg-raised\);\s*box-shadow: var\(--pillow-card\)/);
  assert.match(rule('\\.app\\[data-pane="chat"\\] \\.msg\\.pt-railed'), /border-radius: 22px/);
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

test("the server strip and the glass slab on Home keep the design guide's measures", () => {
  const rule = selector => {
    const at = css.indexOf(`html[data-look="studio"][data-fit="slim"] ${selector} {`);
    assert.ok(at >= 0, selector);
    return css.slice(at, css.indexOf("}", at));
  };
  assert.match(rule('.app[data-pane="list"] > .rail'), /height: 96px;\s+padding: 10px 0 0 10px;/);
  assert.match(rule('.app[data-pane="list"] .rail-item'), /gap: 8px;\s+width: 74px;\s+height: 86px;\s+padding-top: 3px;/);
  assert.match(rule('.app[data-pane="list"] :is(.server-icon, .rail-btn.rail-add, .folder-head)'), /width: 52px;\s+height: 52px;/);
  assert.match(rule('.app[data-pane="list"] .server-icon.active'), /outline: var\(--ring-on\);\s+outline-offset: 2px;/);
  assert.match(rule('.app[data-pane="list"] .rail-item:has(> .server-icon.active) > .count-badge'), /display: none;/);
  assert.match(rule('.app[data-pane="list"] > .sidebar'), /margin: 14px 12px calc\(var\(--hz-line\) \+ 70px\);[\s\S]*border-radius: var\(--radius-xl\);/);
  assert.match(rule('.app[data-pane="list"] > .sidebar > .pt-find'), /right: -4px;\s+top: -16px;[\s\S]*width: 50px;\s+height: 50px;/);
  assert.match(rule('.app[data-pane="list"] .sidebar-header > #server-name'), /font: 700 23px\/1\.45 var\(--font-display\);/);
  assert.match(rule('.app[data-pane="list"][data-tab="home"]:not(.dm-mode) .sidebar .header-btns'), /display: none;/);
  assert.match(rule('.app[data-pane="list"] .sidebar-game-tags'), /height: 18px;\s+margin: 2px 0 0;/);
  assert.match(rule('.app[data-pane="list"] .channel-group.section-head .dm-group-add'), /display: none;/);
  assert.match(rule('.app[data-pane="list"] .channel'), /height: 50px;\s+min-height: 50px;\s+margin: 0;\s+padding: 0 12px;\s+gap: 12px;/);
  assert.match(rule('.app[data-pane="list"] .channel > .pt-count'), /min-width: 24px; height: 24px;/);
  assert.match(rule('.app[data-pane="list"] .channel.voice-channel.pt-busy'), /height: 58px;/);
});

test("Home reads in the guide's words, and the server name opens the server's own menu", () => {
  assert.match(glue, /const STUDIO_HOME_WORDS = \{ "Server Rules": "Rules", "Text Channels": "Text channels", "Voice Channels": "Voice channels" \};/);
  assert.match(glue, /const openMenu = \(\) => openServerMenu\(state\.currentServer, 0, 0\);/);
  assert.match(glue, /openCreateChannel\("text", null\);/);
  assert.match(read("scripts", "app.js"), /function textChannelRow\([\s\S]{0,400}row\.dataset\.channel = channel\.id;/);
});

test("the faces on the room you are in leave you out, and every other room shows everyone", () => {
  const people = [{ id: "me" }, { id: "a" }, { id: "b" }];
  assert.deepEqual(roomFaces(people, "me").map(person => person.id), ["a", "b"]);
  assert.deepEqual(roomFaces(people, null).map(person => person.id), ["me", "a", "b"]);
});

test("the search sheet borrows the app's own search and hands it back when it closes", () => {
  const open = glue.slice(glue.indexOf("function openSearchSheet()"), glue.indexOf("function stowSearchSheet()"));
  const stow = glue.slice(glue.indexOf("function stowSearchSheet()"), glue.indexOf("function wireHomeHead()"));
  assert.match(open, /\$\("pt-search-top"\)\.appendChild\(box\);\s+\$\("pt-search-body"\)\.appendChild\(pop\);\s+openSheet\("pt-search-open"\);/);
  assert.match(open, /\$\("search-input"\)\.focus\(\);/);
  assert.match(stow, /resetSearch\(\);[\s\S]*home\.boxParent\.insertBefore\(\$\("search-box"\), home\.boxNext\);\s+home\.popParent\.insertBefore\(\$\("search-pop"\), home\.popNext\);/);
  assert.match(glue, /if \(studioPhone\.searchHome && !app\.classList\.contains\("pt-search-open"\)\) stowSearchSheet\(\);/);
});

test("keys that act are drawn filled, hanging up is the same mark everywhere, and the wall carries the lamp's band", () => {
  assert.match(css, /\.pt-acts \.act \.i \.i-on \{ display: inline; \}/);
  assert.equal(ROOM_KEYS.find(key => key.id === "leave").icon, "phone-off");
  assert.equal((glue.match(/aria-label="Hang up"><i data-i="phone-off"><\/i>/g) || []).length, 2);
  assert.doesNotMatch(css, /\.hang \.i \{[^}]*rotate/);
  assert.equal((css.match(/linear-gradient\(118deg, rgb\(var\(--lamp\) \/ 0\) 16%, rgb\(var\(--lamp\) \/ \.07\) 24%, rgb\(var\(--lamp\) \/ \.07\) 40%, rgb\(var\(--lamp\) \/ 0\) 48%\)/g) || []).length, 2);
  assert.doesNotMatch(css, /\.(?:count-badge|badge|pt-count)[^{]*\{[^}]*tabular-nums/);
});

test("the name under your server may run past its tile, but never into a neighbour's name or the screen edge", () => {
  assert.equal(TILE_NAME.cells, 1.5);
  assert.equal(tileNameWidth(74, 100, [40, 40]), 111);
  assert.equal(tileNameWidth(74, 2, [7.75]), 78);
  assert.equal(tileNameWidth(74, 100, [0, 30]), 74);
  assert.equal(tileNameWidth(74, -40, []), 74);
  assert.match(glue, /name\.style\.maxWidth = cell \? `\$\{tileNameWidth\(cell, item\.offsetLeft - TILE_NAME\.margin, sides\)\}px` : "";/);
});

test("a light server colour takes dark initials on the tile you are in", () => {
  assert.match(glue, /button\.classList\.toggle\("pt-ink", serverTintIsLight\(button\.style\.getPropertyValue\("--server-tint"\)\)\);/);
  assert.match(css, /\.server-icon\.active\.pt-ink \{ color: var\(--ink-strong\); \}/);
});

test("the search sheet's scope keys are small glass keys with a full-size touch area, the chosen one held", () => {
  const at = css.indexOf('.pt-search-body .search-scope-btn {');
  const rule = css.slice(at, css.indexOf("}", at));
  assert.match(rule, /height: 32px;\s+padding: 0 14px;[\s\S]*background: var\(--k-plain\);/);
  assert.match(css, /\.pt-search-body \.search-scope-btn::after \{ content: ""; position: absolute; inset: -6px 0; \}/);
  assert.match(css, /\.pt-search-body \.search-scope-btn\.active \{ background: var\(--k-latched\); box-shadow: var\(--k-latched-shadow\);/);
});

test("at most three friend requests show as cards, and a plain row counts the rest until it is opened", () => {
  assert.equal(WAIT_CARDS, 3);
  assert.deepEqual(waitCards(1, false), { shown: 1, more: 0 });
  assert.deepEqual(waitCards(3, false), { shown: 3, more: 0 });
  assert.deepEqual(waitCards(7, false), { shown: 3, more: 4 });
  assert.deepEqual(waitCards(7, true), { shown: 7, more: 0 });
  assert.equal(waitMoreWords(4), "4 more requests");
  assert.equal(waitMoreWords(1), "1 more request");
});

test("a conversation's age reads short: minutes, hours, the weekday, then the date", () => {
  const now = Date.parse("2026-11-10T20:00:00Z");
  const ago = minutes => new Date(now - minutes * 60000).toISOString();
  assert.equal(listAge(ago(0), now, "en-US"), "now");
  assert.equal(listAge(ago(2), now, "en-US"), "2m");
  assert.equal(listAge(ago(59), now, "en-US"), "59m");
  assert.equal(listAge(ago(60), now, "en-US"), "1h");
  assert.equal(listAge(ago(23 * 60 + 59), now, "en-US"), "23h");
  assert.match(listAge(ago(3 * 1440), now, "en-US"), /^(Fri|Sat|Sun)$/);
  assert.match(listAge(ago(30 * 1440), now, "en-US"), /^Oct 1[01]$/);
  assert.equal(listAge(null, now, "en-US"), "");
  assert.equal(listAge("never", now, "en-US"), "");
});

test("the list's labels read in sentence case, and Friends drops its count", () => {
  assert.equal(listLabel("Direct Messages"), "Direct messages");
  assert.equal(listLabel("Friends - 2"), "Friends");
  assert.equal(listLabel("Pinned"), "Pinned");
  assert.equal(listLabel("Message Requests - 3"), "Message requests - 3");
  assert.equal(listLabel("Friends of ferris"), "Friends of ferris");
});

test("Messages follows the guide's measures: title, search field, compose key, rows and labels", () => {
  const rule = selector => {
    const at = css.indexOf(`html[data-look="studio"][data-fit="slim"] ${selector} {`);
    assert.notEqual(at, -1, selector);
    return css.slice(at, css.indexOf("}", at));
  };
  assert.match(rule('.app[data-pane="list"]:not([data-tab="home"]) > .pt-title'), /height: 62px;\s+padding: 14px 24px 0;/);
  assert.match(rule('.app[data-pane="list"][data-tab="messages"] > .sidebar'), /margin-top: 14px;/);
  assert.match(rule('.app[data-pane="list"][data-tab="messages"] .sidebar-header'), /padding: 14px 64px 0 12px;/);
  assert.match(rule('.app[data-pane="list"] .dm-search'), /height: 46px;[\s\S]*padding: 0 18px 2px 47px;[\s\S]*font: 400 15\.5px\/1 var\(--font\);/);
  assert.match(rule(".pt-dm-find .i"), /width: 19px; height: 19px;/);
  assert.match(rule('.app.dm-mode[data-pane="list"][data-tab="messages"] > .sidebar > .pt-compose'), /right: -4px;\s+top: -16px;[\s\S]*width: 50px;\s+height: 50px;/);
  assert.match(rule(".pt-compose .i"), /width: 21px; height: 21px;/);
  assert.match(rule('.app[data-pane="list"] .dm-thread'), /height: 58px;\s+min-height: 58px;\s+padding: 0 10px 0 12px;\s+gap: 12px;\s+border-radius: 29px;/);
  assert.match(rule('.app[data-pane="list"] .dm-thread .avatar'), /width: 40px; height: 40px;/);
  assert.match(rule('.app[data-pane="list"] .dm-name'), /font: 600 16px\/1 var\(--font\);/);
  assert.match(rule('.app[data-pane="list"] .dm-preview'), /font: 400 14px\/1\.15 var\(--font\);\s+color: var\(--text-muted\);/);
  assert.match(rule('.app[data-pane="list"] .dm-time'), /font: 400 13px\/1 var\(--font\); color: var\(--text-faint\);/);
  assert.match(rule('.app[data-pane="list"] .dm-thread > .pt-dm-side > .pt-count'), /min-width: 22px; height: 22px; margin: 0; font-size: 12\.5px;/);
  assert.match(rule('.app[data-pane="list"] .dm-thread .presence-dot'), /width: 11px;\s+height: 11px;[\s\S]*box-shadow: 0 0 0 2\.5px var\(--bg-dark\);/);
  assert.match(rule('.app[data-pane="list"] .dm-thread .pt-duo > .avatar'), /width: 28px; height: 28px;/);
  assert.match(rule('.app[data-pane="list"] .channel-group.dm-group'), /height: 32px; padding: 14px 12px 0; align-items: center; font: 600 13px\/1 var\(--font\);/);
  assert.match(css, /\.app\[data-pane="list"\] \.channel-group\.pt-bare,\n[^\n]*\.channel-group\.dm-group > \.dm-group-add \{ display: none; \}/);
});

test("a friend request is a tungsten card with an Accept key and a plain x, and its note takes one more line", () => {
  const rule = selector => {
    const at = css.indexOf(`html[data-look="studio"][data-fit="slim"] .app[data-pane="list"] ${selector} {`);
    assert.notEqual(at, -1, selector);
    return css.slice(at, css.indexOf("}", at));
  };
  const card = rule(".dm-thread.pt-wait");
  assert.match(card, /height: 68px;\s+min-height: 68px;\s+margin: 12px 4px 0;\s+padding: 0 6px 0 14px;\s+gap: 11px;\s+border-radius: 999px;/);
  assert.match(card, /var\(--accent-2\);[\s\S]*color: var\(--accent-2-ink\);/);
  assert.match(rule(".dm-thread.pt-wait + .dm-thread.pt-wait"), /margin-top: 8px;/);
  assert.match(rule(".dm-thread.pt-wait.pt-wait-held"), /display: none;/);
  assert.match(rule(".dm-thread.pt-wait:has(.friend-note:not(.hidden))"), /height: 84px; min-height: 84px;/);
  assert.match(rule(".pt-wait .dm-name"), /font: 700 16px\/1 var\(--font\); color: inherit;/);
  assert.match(rule(".pt-wait .pt-wait-line"), /font: 500 13\.5px\/1 var\(--font\);/);
  assert.match(rule(".pt-wait .friend-note"), /font: 400 13px\/1\.15 var\(--font\); color: inherit;/);
  assert.match(rule(".pt-wait .friend-yes"), /height: 44px;\s+padding: 0 18px;[\s\S]*background: var\(--pillow-top\), var\(--accent-2-ink\);[\s\S]*font: 600 14\.5px\/1 var\(--font\);/);
  assert.match(rule(".pt-wait .friend-no"), /width: 44px;\s+height: 44px;[\s\S]*background: none;/);
  assert.match(rule(".pt-wait-rest"), /height: 50px;\s+padding: 0 12px;/);
});

test("the Messages glue builds the card, the compose key and the side column from the app's own rows", () => {
  const app = read("scripts", "app.js");
  assert.match(glue, /const compose = studioNode\("button", "key sq cream pt-compose", '<i data-i="square-pen"><\/i>'\);/);
  assert.match(glue, /compose\.onclick = \(\) => openNewDm\(\);/);
  assert.match(glue, /field\.placeholder = studioSlim\(\) \? "Find a person or a chat" : field\.dataset\.ptTyped;/);
  assert.match(glue, /note\.before\(el\("div", "pt-wait-line", "wants to be friends"\)\);\s+note\.classList\.toggle\("hidden", !note\.title\);/);
  assert.match(glue, /if \(e\.target\.closest\("button"\)\) return;\s+openMemberCard\(\{ id: row\.dataset\.person, name: row\.querySelector\("\.dm-name"\)\.textContent \}, row\);/);
  assert.match(glue, /const \{ shown, more \} = waitCards\(cards\.length, studioPhone\.waitsOpen\);/);
  assert.match(glue, /if \(thread\) time\.textContent = listAge\(thread\.last_at, now\);/);
  assert.match(glue, /if \(tab === "messages" && state\.view !== "dm"\) await enterDmMode\(false\);/);
  assert.match(glue, /decorateRooms\(\);\s+decorateMessages\(\);/);
  assert.match(app, /async function enterDmMode\(openFirst = true\) \{/);
  assert.match(app, /if \(!openFirst\) state\.currentDm = null;/);
  assert.match(app, /if \(!openFirst\) \{\s+renderDmHome\(\);\s+\} else if \(state\.currentDm\) \{/);
  assert.equal((app.match(/row\.dataset\.dm = thread\.user;/g) || []).length, 2);
  assert.match(app, /row\.dataset\.person = req\.id;/);
  assert.match(app, /row\.dataset\.person = friend\.id;/);
});

test("the Messages tab counts chats with something unread, and Saved is the first row under its label", () => {
  assert.equal(unreadChats([2, 0, 5]), 2);
  assert.equal(unreadChats([2]), 1);
  assert.equal(unreadChats([]), 0);
  assert.match(glue, /const text = badgeText\(unreadChats\(\[\.\.\.unread\.dms\.values\(\), \.\.\.state\.servers\.filter\(server => server\.group\)\.map\(server => serverUnreadCount\(server\.id\)\)\]\)\);/);
  assert.match(glue, /const first = list\.querySelector\(":scope > \.channel-group"\);\s+first\.after\(saved\);/);
  assert.match(glue, /const twice = label\.previousElementSibling === saved && label\.textContent === first\.textContent;/);
});
