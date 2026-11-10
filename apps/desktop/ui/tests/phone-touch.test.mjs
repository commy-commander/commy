import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  PHONE_TABS, HORIZON, HORIZON_SPRINGS, HANG_UP_FUSE_MS, horizonY, horizonPath, horizonLift, springStep,
  phoneTabAfter, hangUpTap, badgeText, roomLine, liveSummary, liveRooms, liveStreams, stackPeople,
  SWIPE, QUICK_REACTIONS, SHEET_ROWS, swipeAxis, replyPull, replyCommits, edgeBackStarts, edgeBackCommits,
  rowPull, rowStaysOpen, sheetRow, quickReactions, chatSubtitle,
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

test("pulling a message right follows the finger, then resists, and replies past the mark", () => {
  assert.equal(replyPull(-20), 0);
  assert.equal(replyPull(30), 30);
  assert.equal(replyPull(SWIPE.replyAt), SWIPE.replyAt);
  assert.ok(replyPull(120) > SWIPE.replyAt && replyPull(120) < 120);
  assert.equal(replyPull(900), SWIPE.replyMax);
  assert.equal(replyCommits(SWIPE.replyAt - 1), false);
  assert.equal(replyCommits(SWIPE.replyAt), true);
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
