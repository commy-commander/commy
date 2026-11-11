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
  WAIT_CARDS, waitCards, waitMoreWords, listAge, listLabel, unreadChats, ROOM_LAMPS, TRAY_PULL, trayOpens, canOfferShare, MIRROR_FRAME_MS, roomTileStreams, shareViewers, roomStage, audienceLabel, WATCH_STEPS, watchStack, STREAM_TOOLS_MS, streamToolsStay, trayTap, trayNeedsStep, NOTIFY_PICKS, earlyBuild, chatBanner, layerStep, inviteWarningParts, faceTapAuthor,
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
  assert.equal(canOfferShare({ mobileApp: true, canShare: true, iPhone: true }), false);
  assert.equal(canOfferShare({ mobileApp: false, canShare: false, iPhone: true }), false);
  assert.equal(canOfferShare({ mobileApp: true, canShare: false, iPhone: false }), false);
  assert.equal(canOfferShare({ mobileApp: true, canShare: true, iPhone: false }), true);
  assert.equal(canOfferShare({ mobileApp: false, canShare: false, iPhone: false }), true);
  assert.equal((glue.match(/share screen/gi) || []).length, 1);
  assert.match(glue, /share\.classList\.toggle\("hidden", !canOfferShare\(\{ mobileApp: body\.contains\("mobile-app"\), canShare: body\.contains\("can-share-screen"\), iPhone: \/iPhone\|iPod\/\.test\(navigator\.userAgent\) \}\)\);/);
  assert.match(css, /\.app\[data-pane="list"\] \.sidebar \.voice-panel \{ display: none; \}/);
  assert.doesNotMatch(css, /pt-voice-open [^{]*\.voice-panel/);
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
  assert.equal(SWIPE.rowOpen, 112);
  assert.ok(SWIPE.rowOpen >= 14 + 2 * 44 + 8);
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
  assert.match(css, /\.pt-hold \.rx-key \{\n  width: (4[4-9]|[5-9]\d)px;\n  height: (4[4-9]|[5-9]\d)px;/);
  assert.equal((glue.match(/class="key sq tap (primary )?act (read|mute)"/g) || []).length, 2);
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
  assert.match(css, /\.pt-watch \.ctl \.key\.sq \{ width: (4[4-9]|[5-9]\d)px; height: (4[4-9]|[5-9]\d)px;/);
  assert.match(css, /\.pt-watch \.wbar \.hang \{[^}]*width: (4[4-9]|[5-9]\d)px; height: (4[4-9]|[5-9]\d)px;/);
  assert.match(css, /\.stage-btn \{ flex: none; width: (4[4-9]|[5-9]\d)px; height: (4[4-9]|[5-9]\d)px; \}/);
  assert.match(glue, /classList\.toggle\("theme-dark", full\)/);
});

test("the phone's chat shows no stream or camera tiles above it; streams play in the room and on the stream screen", () => {
  const P = 'html[data-look="studio"][data-fit="slim"]';
  assert.ok(css.includes(`${P} .app[data-pane="chat"]:not(.pt-stream-full) .stage { display: none; }`));
  assert.ok(css.includes(`${P} .app[data-pane="chat"] .main > .cam-grid { display: none; }`));
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

test("the soundboard tray stays open for the room's own keys, and any other tap in the room only closes it", () => {
  assert.equal(trayTap(true, "key"), "stay");
  assert.equal(trayTap(true, "room"), "close");
  assert.equal(trayTap(true, "tray"), "pass");
  assert.equal(trayTap(true, "outside"), "pass");
  for (const place of ["key", "room", "tray", "outside"]) assert.equal(trayTap(false, place), "pass");
  assert.match(glue, /function trayPlace\(target\) \{\n  if \(!target\.closest\) return "outside";\n  if \(target\.closest\("\[data-room-key\]"\)\) return "key";\n  if \(target\.closest\("\.soundboard-pop"\)\) return "tray";\n  return target\.closest\("#pt-room, \.hzn"\) \? "room" : "outside";/);
  assert.match(glue, /window\.addEventListener\("mousedown", e => \{\n    if \(studioSlim\(\) && trayShown\(\) && trayTap\(true, trayPlace\(e\.target\)\) === "stay"\) e\.stopPropagation\(\);\n  \}, true\);/);
  assert.match(glue, /if \(!studioSlim\(\) \|\| trayTap\(wasOpen, trayPlace\(e\.target\)\) !== "close"\) return;\n    e\.preventDefault\(\);\n    e\.stopPropagation\(\);\n    \$\("soundboard-pop"\)\.classList\.toggle\("hidden", true\);\n  \}, true\);/);
  assert.match(glue, /function trayShown\(\) \{\n  const pop = \$\("soundboard-pop"\);\n  return pop\.classList\.contains\("pt-tray"\) && !pop\.classList\.contains\("hidden"\);/);
});

test("going back with the tray open closes only the tray, and leaving the room steps past both", () => {
  assert.equal(trayNeedsStep({ pane: "list", ptSheet: "pt-voice-open" }), true);
  assert.equal(trayNeedsStep({ pane: "list", ptSheet: "pt-voice-open", ptTray: true }), false);
  assert.equal(trayNeedsStep({ pane: "list", ptSheet: "pt-hold-open" }), false);
  assert.equal(trayNeedsStep({ pane: "list" }), false);
  assert.equal(trayNeedsStep(null), false);
  assert.match(glue, /history\.pushState\(\{ \.\.\.history\.state, ptTray: true \}, ""\);\n    studioPhone\.trayStep = true;/);
  assert.match(glue, /\} else if \(!tray && studioPhone\.trayStep\) \{\n    studioPhone\.trayStep = false;\n    if \(history\.state && history\.state\.ptTray\) history\.back\(\);/);
  assert.match(glue, /if \(tray !== was\) holdTrayStep\(tray\);/);
  assert.match(glue, /if \(!\(history\.state && history\.state\.ptTray\)\) \{\n      studioPhone\.trayStep = false;\n      if \(trayShown\(\)\) \$\("soundboard-pop"\)\.classList\.toggle\("hidden", true\);\n    \}\n    dropSheets\(\);/);
  assert.match(glue, /if \(history\.state\.ptTray\) \{\n      studioPhone\.trayStep = false;\n      history\.go\(-2\);\n    \} else history\.back\(\);/);
});

test("server names under the tiles keep their descenders", () => {
  assert.match(css, /\.rail-folder > \.pt-name \{\n  max-width: 74px;\n  overflow: hidden;\n  text-overflow: ellipsis;\n  white-space: nowrap;\n  margin-block: -2px;\n  padding-block: 2px;/);
});

test("a swiped channel row shows two round keys with no words, and the row runs off the pane's edge", () => {
  const acts = glue.match(/studioNode\("div", "pt-acts", `([^`]*)`\)/)[1];
  assert.match(acts, /<button type="button" class="key sq tap primary act read" aria-label="Mark read"><i data-i="check-check"><\/i><\/button>/);
  assert.match(acts, /<button type="button" class="key sq tap act mute" aria-label="\$\{muted \? "Unmute" : "Mute"\}"><i data-i="\$\{muted \? "bell" : "bell-off"\}"><\/i><\/button>/);
  assert.doesNotMatch(acts.replace(/aria-label="[^"]*"/g, "").replace(/\$\{[^}]*\}/g, ""), />[^<]+</);
  assert.match(css, /\.pt-acts \{[^}]*left: 14px;[^}]*gap: 8px;/);
  assert.match(css, /\.pt-acts \.act \.i \{ width: 20px; height: 20px; \}/);
  assert.match(css, /\.pt-acts \.act\.read \{ --k: var\(--swipe-done\); box-shadow: var\(--pillow-sm\); \}/);
  assert.match(css, /\[data-theme="light"\] \.pt-acts \.act\.mute \{ box-shadow: var\(--pillow-sm\); \}/);
  assert.match(css, /\.channel\.pt-open \{ width: calc\(100% - 44px\); padding-right: 92px; \}/);
  assert.match(css, /\.channel-scroll:has\(\.channel\.pt-dragging\) \{ overflow-x: hidden; clip-path: inset\(0 12px 0 0\); \}/);
});

test("keys that act are drawn filled, hanging up is the same mark everywhere, and the wall carries the lamp's band", () => {
  assert.equal(ROOM_KEYS.find(key => key.id === "leave").icon, "phone-off");
  assert.equal((glue.match(/aria-label="Hang up"><i data-i="phone-off"><\/i>/g) || []).length, 2);
  assert.doesNotMatch(css, /\.hang \.i \{[^}]*rotate/);
  assert.equal((css.match(/linear-gradient\(118deg, rgb\(var\(--lamp\) \/ 0\) 16%, rgb\(var\(--lamp\) \/ \.07\) 24%, rgb\(var\(--lamp\) \/ \.07\) 40%, rgb\(var\(--lamp\) \/ 0\) 48%\)/g) || []).length, 4);
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

const slim = 'html\\[data-look="studio"\\]\\[data-fit="slim"\\]';
const rule = (selector, body) => new RegExp(`${slim}${selector} \\{[^}]*${body}`);

test("the room keys carry lamps, deafen shows a slashed headphone, and the camera key knows when it is on", () => {
  assert.deepEqual(ROOM_LAMPS, ["sounds", "camera", "mic", "deafen"]);
  assert.match(css, rule(' \\.hzn-room \\.key\\[data-room-key="mic"\\]:not\\(\\.hand\\.is-on\\) \\.led', 'display: none;'));
  assert.equal(roomKeyState("deafen", { deafened: true }).icon, "headphone-off");
  assert.equal(roomKeyState("deafen", {}).icon, "headphones");
  assert.equal(roomKeyState("camera", { camera: true }).on, true);
  assert.equal(roomKeyState("camera", {}).on, false);
  assert.match(glue, /ROOM_LAMPS\.includes\(key\.id\) \? '<i class="led"><\/i>' : ""/);
  assert.match(css, rule(' \\.hzn-room \\.key \\.led', 'background: var\\(--green\\);'));
  assert.match(css, rule(' \\.hzn-room \\.key\\[data-room-key="sounds"\\]:not\\(\\.is-down\\) \\.led', 'background: var\\(--text-faint\\);'));
  assert.match(css, rule(' \\.hzn-room \\.key\\[data-room-key="camera"\\]:not\\(\\.is-on\\) \\.led', 'display: none;'));
});

test("the room keys keep the guide's sizes, and nothing but leaving is red", () => {
  assert.match(css, rule(' \\.hzn-room \\.k \\.key', 'bottom: 49px; width: 52px; height: 52px;'));
  assert.match(css, rule(' \\.hzn-room \\.k\\.big \\.key', 'bottom: 56px; width: 62px; height: 62px;'));
  assert.match(css, rule(' \\.hzn-room \\.key \\.i', 'width: 22px; height: 22px;'));
  assert.match(css, rule(' \\.hzn-room \\.k\\.big \\.key \\.i', 'width: 27px; height: 27px;'));
  assert.match(css, rule(' \\.hzn-room \\.key\\.is-down', 'background: var\\(--k-latched\\); box-shadow: var\\(--k-latched-shadow\\);'));
  assert.match(css, rule(' \\.hzn-room \\.key\\.is-off', 'background: var\\(--glass-hi\\), transparent;'));
  for (const found of css.matchAll(/\.hzn-room [^{]*\{[^}]*\}/g)) {
    if (/danger|--red/.test(found[0])) assert.match(found[0], /\.hang|\.ask|\.hzn-fuse/, found[0].slice(0, 80));
  }
});

test("the room header is the guide's: a chevron, the name, who is here, and the quality mark as one key", () => {
  assert.match(css, rule(' \\.pt-room-head', 'gap: 4px;\\s+height: 60px;\\s+padding: 0 14px 0 6px;'));
  assert.match(css, rule(' \\.pt-room-head \\.t small', 'font: 400 13\\.5px/1 var\\(--font\\);'));
  assert.match(css, rule(' \\.pt-room-head \\.sig', '[\\s\\S]*height: 34px;\\s+padding: 0 6px;[\\s\\S]*font: 600 13\\.5px/1 var\\(--font\\);'));
  assert.match(css, rule(' \\.pt-room-head \\.sig::after', 'inset: -5px 0;'));
  assert.match(glue, /<button type="button" class="sig" id="pt-room-sig" aria-haspopup="dialog" aria-expanded="false">\$\{studioDockSignalMarkup\(\)\}<span id="pt-room-sig-word"><\/span><\/button>/);
  assert.doesNotMatch(glue, /pt-room-more|pt-room-opts/);
});

test("the quality mark opens a Voice sheet with the connection and the app's own device choices", () => {
  assert.match(glue, /<h3>Voice<\/h3>/);
  for (const [caret, word] of [["mute-caret", "Microphone"], ["deafen-caret", "Speaker"], ["camera-caret", "Camera"]]) {
    assert.match(glue, new RegExp(`data-caret="${caret}"><i data-i="[a-z-]+"></i><span class="l">${word}</span><small id="pt-voice-[a-z]+"></small>`));
  }
  assert.match(glue, /\$\(row\.dataset\.caret \|\| row\.dataset\.press\)\.click\(\);/);
  assert.match(glue, /id="pt-voice-share" data-press="share-btn"/);
  assert.match(glue, /id="pt-voice-act" data-press="vp-activities"><i data-i="gamepad-2"><\/i>/);
  assert.match(glue, /act\.classList\.toggle\("hidden", \$\("vp-activities"\)\.classList\.contains\("hidden"\)\);/);
  assert.match(glue, /conn\.querySelector\("b"\)\.textContent = words\.line;\n\s+conn\.querySelector\("small"\)\.textContent = words\.sub;/);
  assert.match(css, rule(' \\.pt-voice-scrim', 'z-index: 6;'));
  assert.match(css, rule(' \\.pt-voice-sheet', '[\\s\\S]*z-index: 7;[\\s\\S]*border-radius: var\\(--radius-sheet\\);'));
  assert.match(css, rule(' \\.pt-room\\.opts ~ :is\\(\\.pt-voice-scrim, \\.pt-voice-sheet\\)', 'display: block;'));
  assert.match(css, rule(' \\.pt-voice-sheet h3', '[^}]*font: 700 17px/1 var\\(--font-display\\);'));
});

test("the phone's profile editor finds the server tag's hint wherever the settings page wraps the picker", () => {
  assert.match(glue, /\$\("pt-prof-tag-hint"\)\.textContent = \$\("account-server-tag"\)\.closest\("\.field-label"\)\.querySelector\("\.setting-sub"\)\.textContent;/);
  const start = html.indexOf('id="account-server-tag"');
  const label = html.lastIndexOf('<label class="field-label"', start);
  assert.ok(label > 0 && html.indexOf("</label>", label) > html.indexOf('class="setting-sub"', start));
});

test("a drive on the phone app drops the keys that cannot work there and shows one quiet line", () => {
  const P = 'html[data-look="studio"][data-fit="slim"]';
  assert.ok(css.includes(`${P} body.mobile-app .drive-tools,\n${P} .drive-desktop-note { display: none; }`));
  assert.ok(css.includes(`${P} .drive-empty > span { max-width: 280px;`));
  assert.match(read("scripts", "app.js"), /: studioLook\(\) \? \$\("drive-desktop-note"\)\.textContent : "Open this channel in the desktop app to see its files\."\)\);/);
});

test("a stream you have not opened is a dark glass tile with the sharer's face and one Watch key", () => {
  assert.match(glue, /<span class="key primary go">Watch<\/span>/);
  assert.doesNotMatch(glue, /Tap to watch/);
  assert.match(css, rule(' \\.pt-stream', '[\\s\\S]*flex: 1 0 calc\\(50% - 4px\\);[\\s\\S]*height: 206px;[\\s\\S]*border-radius: 28px;'));
  assert.match(css, rule(' \\.pt-stream \\.go', 'height: 44px;'));
  assert.match(css, rule(' \\.pt-room-streams', 'display: flex; gap: 8px;'));
  assert.match(css, rule(' \\.pt-room-streams:has\\(> :nth-child\\(3\\)\\)', '[^}]*overflow-x: auto;'));
});

test("people sit four across on raised tiles, and the mute mark is a small well in the corner", () => {
  assert.match(css, rule(' \\.pt-room-people', 'display: grid; grid-template-columns: repeat\\(4, minmax\\(0, 1fr\\)\\); gap: 8px;'));
  assert.match(css, rule(' \\.ptile', '[\\s\\S]*height: 106px;\\s+border-radius: 26px;[\\s\\S]*box-shadow: var\\(--pillow-card\\);\\s+font: 600 13\\.5px/1 var\\(--font\\);'));
  assert.match(css, rule(' \\.ptile \\.avatar', 'width: 48px; height: 48px;'));
  assert.match(css, rule(' \\.ptile \\.mo \\.i', 'width: 15px; height: 15px;'));
  assert.match(css, rule(' \\.ptile\\.speaking \\.avatar', 'box-shadow: 0 0 0 2\\.5px var\\(--bg-raised\\), 0 0 0 5px var\\(--speaking\\);'));
});

test("the sound tray sits on the line with six pads, and a pull up opens every sound", () => {
  assert.equal(TRAY_PULL, 40);
  assert.equal(trayOpens(-40), true);
  assert.equal(trayOpens(-39), false);
  assert.equal(trayOpens(60), false);
  assert.equal(trayOpens(undefined), false);
  assert.match(css, rule(' \\.soundboard-pop\\.pt-tray:not\\(\\.hidden\\)', '[\\s\\S]*bottom: var\\(--hz-line\\);\\s+height: 296px;[\\s\\S]*padding: 0 14px;\\s+border-radius: 34px 34px 0 0;'));
  assert.match(css, rule(' \\.pt-tray \\.sb-grid', 'display: grid; grid-template-columns: repeat\\(3, minmax\\(0, 1fr\\)\\); align-content: start; gap: 10px 8px;'));
  assert.match(css, rule(' \\.pt-tray:not\\(\\.pt-tray-full\\) \\.sb-sound:nth-of-type\\(n\\+7\\)', 'display: none;'));
  assert.match(css, rule(' \\.pt-tray:not\\(\\.pt-tray-full\\) :is\\(\\.sb-section, \\.sb-empty, \\.sb-muted, \\.pt-tray-add\\)', 'display: none;'));
  assert.match(css, rule(' \\.pt-tray \\.sb-add', 'display: none;'));
  assert.match(css, rule(' \\.pt-tray \\.pt-tray-grab::before', 'content: ""; width: 38px; height: 5px;'));
});

test("the tray is the app's own soundboard, moved into the room and put back when it closes", () => {
  assert.match(glue, /const tray = studioSlim\(\) && roomOpen\(\) && !pop\.classList\.contains\("hidden"\);/);
  assert.match(glue, /pop\.classList\.toggle\("pt-tray-full", false\);/);
  assert.doesNotMatch(glue, /classList\.remove\("pt-tray-full"\)/);
  assert.match(glue, /studioPhone\.trayHome = \{ parent: pop\.parentNode, next: pop\.nextSibling \};\n\s+\$\("pt-room"\)\.appendChild\(pop\);/);
  assert.match(glue, /studioPhone\.trayHome\.parent\.insertBefore\(pop, studioPhone\.trayHome\.next\);/);
  assert.match(glue, /el\("small", "pt-tray-line", "The whole room hears these\."\)/);
  assert.match(glue, /add\.classList\.toggle\("hidden", \$\("sb-add"\)\.classList\.contains\("hidden"\)\);\n\s+add\.onclick = \(\) => \$\("sb-add"\)\.click\(\);/);
  assert.match(glue, /if \(!open\) \$\("soundboard-pop"\)\.classList\.toggle\("hidden", true\);/);
  assert.match(glue, /full \? sheetCloses\(dx\) : swipe\.up && trayOpens\(dx\)/);
});

test("the stream you are watching plays in its tile and carries its own stop key", () => {
  assert.match(glue, /<span class="x" role="button" tabindex="0" aria-label="Stop watching"><i data-i="x"><\/i><\/span>/);
  assert.match(glue, /tile\.querySelector\("\.x"\)\.onclick = e => \{\n\s+e\.stopPropagation\(\);\n\s+stopWatching\(\);/);
  assert.match(css, rule(' \\.pt-stream\\.w \\.max', 'right: 58px;'));
  assert.ok(MIRROR_FRAME_MS >= 16 && MIRROR_FRAME_MS <= 100);
  assert.match(glue, /if \(stream\.watching\) \{\n\s+const pic = el\("canvas", "pic"\);/);
  assert.match(glue, /const crop = frame\.classList\.contains\("blank"\) \? null : coverCrop\(frame\.width, frame\.height, box\.width, box\.height\);\n\s+if \(!crop\) return;/);
  assert.match(glue, /drawImage\(frame, crop\.sx, crop\.sy, crop\.sw, crop\.sh, 0, 0, width, height\);\n\s+pic\.parentNode\.classList\.toggle\("on", true\);/);
  assert.match(glue, /if \(!pic \|\| !roomOpen\(\)\) return;\n\s+studioPhone\.mirror = requestAnimationFrame\(mirrorStream\);/);
  assert.match(css, rule(' \\.pt-stream\\.on :is\\(\\.avatar, \\.go\\)', 'display: none;'));
  assert.match(css, rule(' \\.pt-stream \\.pic', 'position: absolute; inset: 0; width: 100%; height: 100%; border-radius: inherit; visibility: hidden;'));
});

test("your own share is a tile like any other, with your face and one Stop key, and nothing sits in rows above the keys", () => {
  const others = [{ user: "m1", name: "ferris", title: "ferris's screen", watching: false }];
  assert.deepEqual(roomTileStreams(others, { userId: "me", name: "commander", sharing: false }), others);
  assert.deepEqual(roomTileStreams(others, { userId: "me", name: "commander", sharing: true, viewers: 3 })[0], { user: "me", name: "commander", title: "Your screen", short: "You", own: true, watching: false, viewers: 3 });
  assert.equal(roomTileStreams(others, { userId: "me", sharing: true }).length, 2);
  assert.deepEqual(roomTileStreams(null, {}), []);
  assert.match(glue, /<span class="stopk" role="button" tabindex="0">Stop<\/span>/);
  assert.match(glue, /tile\.querySelector\("\.stopk"\)\.onclick = \(\) => \$\("vp-stop-share"\)\.click\(\);/);
  assert.match(css, rule(' \\.pt-stream \\.stopk', '[^}]*padding: 0 16px; border-radius: 999px; font: 600 13\\.5px/1 var\\(--font\\);'));
  for (const found of css.matchAll(/\.pt-stream[^{]*\.stopk[^{]*\{[^}]*\}/g)) assert.doesNotMatch(found[0], /danger|--red/);
});

test("with two or more streams every tile is a small sixteen by nine one", () => {
  const small = ' \\.pt-room-streams:has\\(> :nth-child\\(2\\)\\) \\.pt-stream';
  assert.match(css, rule(small, '[\\s\\S]*height: auto;\\s+aspect-ratio: 16 / 9;'));
  assert.match(css, rule(`${small} \\.avatar`, 'width: 40px; height: 40px;'));
  assert.match(css, rule(`${small} \\.go`, 'height: 32px;'));
});

test("the big key is the one you hold to talk, and in a stage audience it raises your hand", () => {
  assert.deepEqual(roomKeyState("mic", { talk: true }), { down: false, mode: "talk", held: false, icon: "mic", label: "Hold to talk", word: "Hold to talk" });
  assert.equal(roomKeyState("mic", { talk: true, talking: true }).held, true);
  assert.deepEqual(roomKeyState("mic", { talk: true, muted: true }), { down: true, icon: "mic-off", label: "Unmute" });
  assert.deepEqual(roomKeyState("mic", { hand: { raised: false }, talk: true }), { down: false, mode: "hand", icon: "hand", label: "Raise hand", word: "Raise hand" });
  assert.deepEqual(roomKeyState("mic", { hand: { raised: true } }), { down: true, mode: "hand", icon: "hand", label: "Lower hand", word: "Lower hand" });
  assert.match(glue, /if \(mode === "talk"\) return;\n\s+if \(mode === "hand"\) \$\("vp-hand"\)\.click\(\);\n\s+else if \(id === "mic"\) \$\("mute-btn"\)\.click\(\);/);
  assert.match(glue, /if \(!key\.classList\.contains\("talk"\)\) return;\n\s+key\.setPointerCapture\(e\.pointerId\);\n\s+holdTalk\(true\);/);
  assert.match(glue, /for \(const type of \["pointerup", "pointercancel", "lostpointercapture"\]\) key\.addEventListener\(type, \(\) => holdTalk\(false\)\);/);
  assert.match(glue, /invoke\("press_talk", \{ pressed: down \}\)/);
  assert.match(glue, /key\.classList\.toggle\("cream", now\.mode !== "hand"\);/);
  assert.match(glue, /if \(word\) word\.textContent = now\.word \|\| "Mic";/);
});

test("a pad on the tray is its name alone, and a green bar runs under it when you play it", () => {
  assert.match(css, rule(' \\.pt-tray \\.sb-sound > svg', 'display: none;'));
  assert.match(css, rule(' \\.pt-tray \\.sb-sound::after', 'content: ""; position: absolute; left: 24px; right: 24px; bottom: 6px; height: 3px; border-radius: 2px; background: var\\(--green\\); transform: scaleX\\(0\\);'));
  assert.match(css, rule(' \\.pt-tray \\.sb-sound\\.playing::after', 'animation: pt-pad 1200ms linear;'));
  assert.match(glue, /if \(!pad \|\| pad\.disabled \|\| Date\.now\(\) < soundboardCooldown\) return;\n\s+pad\.classList\.toggle\("playing", true\);/);
  assert.match(css, rule(' \\.pt-stream \\.max \\.i\\.i-g', 'stroke: currentColor; stroke-width: 9;'));
  assert.match(css, rule(' \\.pt-tray :is\\(\\.sb-sound, \\.pt-tray-add\\)', '[\\s\\S]*justify-content: center;'));
});

test("your own tile counts who is watching, and says You when it is small", () => {
  assert.equal(shareViewers("3 watching"), 3);
  assert.equal(shareViewers("1 watching"), 1);
  assert.equal(shareViewers("3 watching", true), 0);
  assert.equal(shareViewers(""), 0);
  assert.equal(roomTileStreams([], { userId: "me", sharing: true })[0].viewers, 0);
  assert.match(glue, /<span class="eye by\$\{stream\.viewers \? "" : " hidden"\}"><i data-i="eye"><\/i><b>\$\{stream\.viewers\}<\/b><\/span>/);
  assert.match(glue, /replaceChildren\(el\("span", "full", stream\.title\), el\("span", "short", stream\.short\)\);/);
  const small = ' \\.pt-room-streams:has\\(> :nth-child\\(2\\)\\) \\.pt-stream';
  assert.match(css, rule(' \\.pt-stream \\.eye', '[^}]*font: 600 13px/1 var\\(--font\\); color: var\\(--video-ink\\);'));
  assert.match(css, rule(' \\.pt-stream :is\\(\\.eye\\.by, \\.short\\)', 'display: none;'));
  assert.match(css, rule(`${small} \\.who \\.eye`, 'display: none;'));
  assert.match(css, rule(`${small} \\.eye\\.by:not\\(\\.hidden\\)`, 'display: flex;'));
  assert.match(css, rule(`${small} \\.full`, 'display: none;'));
});

test("while you share, the Voice sheet carries the switch that lets viewers draw", () => {
  assert.match(glue, /id="pt-voice-ink" role="switch" aria-checked="false"><i data-i="pencil"><\/i><span class="l">Let viewers draw<\/span><i class="tog"><\/i>/);
  assert.ok(glue.indexOf('id="pt-voice-share"') < glue.indexOf('id="pt-voice-ink"') && glue.indexOf('id="pt-voice-ink"') < glue.indexOf('id="pt-voice-act"'));
  assert.match(glue, /setStreamInk\(!state\.shareInk\)\.then\(paintVoiceOptions\);/);
  assert.match(glue, /ink\.classList\.toggle\("hidden", \$\("vp-ink-row"\)\.classList\.contains\("hidden"\)\);/);
});

test("on a stage the speakers are the tiles and the audience sits under them, raised hands first", () => {
  const tiles = [{ id: "m1", label: "ferris" }, { id: "m2", label: "juno" }, { id: "m3", label: "quill" }, { id: "me", label: "You" }];
  const stage = roomStage({ kind: "stage", speakers: ["m1"], hands: ["me", "m3"] }, tiles);
  assert.deepEqual(stage.tiles.map(tile => tile.id), ["m1"]);
  assert.deepEqual(stage.audience.map(person => [person.id, person.hand]), [["me", true], ["m3", true], ["m2", false]]);
  assert.deepEqual(roomStage({ kind: "voice" }, tiles), { tiles, audience: [] });
  assert.deepEqual(roomStage(null, tiles), { tiles, audience: [] });
  assert.equal(audienceLabel(14), "Audience 14");
  assert.match(glue, /\$\("pt-room-aud"\)\.classList\.toggle\("hidden", !audience\.length\);/);
  assert.match(glue, /roomHeading\(\{ serverName: vs\.name, count: everyone\.length, call \}\)/);
  assert.match(css, rule(' \\.pt-aud-chip', '[^}]*height: 32px;'));
  assert.match(css, rule(' \\.pt-aud-chip \\.avatar', 'flex: none; width: 24px; height: 24px;'));
  assert.match(css, rule(' \\.pt-aud-chip \\.dot', '[^}]*background: var\\(--accent-2\\);'));
  assert.match(css, rule(' \\.pt-aud-chips', 'display: flex; flex-wrap: wrap;'));
  assert.match(css, rule(' \\.pt-tray \\.sb-sound > \\.custom-emoji', 'flex: none; width: 16px; height: 16px;'));
});

test("over a stream the faces give way first, then the server name, and the title is cut last", () => {
  assert.deepEqual(WATCH_STEPS, [{ faces: 4, server: true }, { faces: 3, server: true }, { faces: 1, server: true }, { faces: 1, server: false }]);
  const people = [{ id: "a", name: "ferris" }, { id: "b", name: "quill" }, { id: "c", name: "juno" }, { id: "d", name: "sprocket" }, { id: "e", name: "nova" }];
  assert.deepEqual(watchStack(people, WATCH_STEPS[0], "d"), { one: false, shown: people.slice(0, 4), more: 1 });
  assert.equal(watchStack(people, WATCH_STEPS[1], "d").more, 2);
  const one = watchStack(people, WATCH_STEPS[2], "d");
  assert.equal(one.one, true);
  assert.equal(one.more, 4);
  assert.deepEqual(one.shown.filter(person => person.lead).map(person => person.id), ["d"]);
  assert.equal(watchStack(people, WATCH_STEPS[2], "gone").shown[0].lead, true);
  assert.deepEqual(watchStack([], WATCH_STEPS[2], "d"), { one: true, shown: [], more: 0 });
  assert.equal(watchPlace({ channelName: "Lounge", name: "Pixel Pioneers" }, false), "Lounge");
  assert.match(glue, /for \(const step of WATCH_STEPS\) \{[\s\S]*?if \(title\.scrollWidth <= title\.clientWidth && place\.scrollWidth <= place\.clientWidth\) break;/);
  assert.match(css, rule(" \\.pt-watch \\.wcard \\.t", "font: 700 15px\\/1 var\\(--font-display\\);"));
  assert.match(css, rule(" \\.pt-watch \\.stack \\.avatar\\.speaking", "width: 36px; height: 36px;"));
  assert.match(css, rule(" \\.pt-watch \\.stack\\.one \\.avatar:not\\(\\.speaking\\):not\\(\\.lead\\)", "display: none;"));
  assert.match(css, rule(" \\.pt-watch \\.stack\\.one:has\\(\\.avatar\\.speaking\\) \\.avatar\\.lead:not\\(\\.speaking\\)", "display: none;"));
  assert.match(css, rule(" \\.pt-watch \\.stack \\.stack-more", "min-width: 30px; height: 30px;"));
});

test("the chevron over a stream goes back to the room, and the stream tools wait for a tap", () => {
  assert.match(glue, /e\.target\.closest\("#pt-watch-down"\)\) \{\s+disarmHangUp\(\);\s+backToRoom\(\);/);
  assert.match(glue, /aria-label="Back to the room"/);
  assert.equal(STREAM_TOOLS_MS, 4000);
  assert.equal(streamToolsStay("pen"), true);
  assert.equal(streamToolsStay("point"), true);
  assert.equal(streamToolsStay(""), false);
  assert.equal(streamToolsStay(undefined), false);
  assert.match(glue, /showStreamTools\(streamToolsStay\(\$\("stage-ink"\)\.dataset\.tool\)\);/);
  assert.match(css, rule(" \\.app\\.pt-stream-full \\.stage-controls", "left: 14px;[^}]*justify-content: flex-start;[^}]*opacity: 0;[^}]*pointer-events: none;"));
  assert.match(css, rule(" \\.app\\.pt-stream-full\\.pt-stream-tools \\.stage-controls", "opacity: 1; visibility: visible; pointer-events: auto;"));
  assert.match(css, rule(" \\.app\\.pt-stream-full :is\\(#stage-focus, #stage-fullscreen, #stage-grid, #video-close\\)", "display: none;"));
  assert.match(css, rule(' \\.app\\.pt-stream-full\\[data-pane="chat"\\] \\.stage:not\\(\\.hidden\\)', "background: var\\(--bg-darkest\\);"));
});

test("the room stands in the same lit room as Home, its line rises wide under the Mic, and a muted mark is the filled one", () => {
  assert.equal(HORIZON.roomSpread, 35);
  assert.equal(HORIZON.roomHot, 70);
  assert.equal(horizonY(300, 300, 9, null, 0, HORIZON.roomSpread), 21);
  assert.ok(horizonY(335, 300, 9, null, 0, HORIZON.roomSpread) < horizonY(335, 300, 9, null, 0));
  assert.match(glue, /horizonPath\(width, studioPhone\.centre, studioPhone\.swell, null, 0, HORIZON\.roomSpread\)/);
  assert.match(css, rule(' \\.app\\[data-pane="list"\\]\\.pt-voice-open > \\.pt-room', "radial-gradient\\(460px 420px at 18% 6%, var\\(--wall-lit\\), transparent 72%\\)"));
  assert.match(css, rule(' \\.app\\[data-pane="list"\\]\\.pt-voice-open > \\.pt-room::after', "background: var\\(--grain\\);"));
  assert.match(css, rule(" \\.ptile \\.mo \\.i \\.i-on", "display: inline;"));
  assert.match(css, rule(" \\.pt-stream::before", "linear-gradient\\(172deg, var\\(--rim\\) 0%"));
  assert.match(css, rule(" \\.pt-room-head \\.t b > span:first-child", "margin: -\\.2em 0; padding: \\.2em 0;"));
});

test("over a stream the pill carries the connection bars, and only the bars take the colour", () => {
  assert.match(glue, /<span class="sg" id="pt-watch-sig">\$\{studioDockSignalMarkup\(\)\}<\/span>/);
  assert.match(glue, /\$\("pt-watch-sig"\)\.dataset\.quality = \$\("vp-signal"\)\.dataset\.quality \|\| "good";/);
  assert.match(css, rule(' \\.pt-watch \\.wcard \\.s \\.sg\\[data-quality="bad"\\] \\.vp-bars', "color: var\\(--signal-bad\\);"));
  assert.doesNotMatch(css, /\.pt-voice-sheet \.conn\[data-quality[^{]*\] b \{/);
});

test("over a stream the side keys and the bar sit where the guide draws them, and only Deafen has a lamp", () => {
  assert.match(glue, /id === "deafen" \? '<i class="led"><\/i>' : ""/);
  assert.match(css, rule(" \\.pt-watch \\.winfo", "left: 14px;\\s+right: 14px;\\s+top: calc\\(env\\(safe-area-inset-top\\) \\+ 8px\\);"));
  assert.match(css, rule(" \\.pt-watch \\.ctl", "bottom: calc\\(var\\(--hz-safe\\) \\+ 66px\\);"));
  assert.match(css, rule(" \\.pt-watch \\.bubbles", "bottom: calc\\(var\\(--hz-safe\\) \\+ 66px\\);"));
  assert.match(css, rule(" \\.pt-watch \\.wbar", "bottom: calc\\(var\\(--hz-safe\\) - 4px\\);"));
  assert.match(css, rule(" \\.pt-watch \\.ctl \\.key \\.led", "background: var\\(--green\\);"));
  assert.match(css, rule(" \\.pt-watch \\.ctl \\.key\\.is-off", "background: var\\(--glass-hi\\), transparent;"));
  assert.match(css, rule(" \\.pt-watch \\.wfield", "padding: 0 22px;"));
});

test("an audience chip is flat: no line and no shadow", () => {
  const at = css.indexOf('html[data-look="studio"][data-fit="slim"] .pt-aud-chip {');
  assert.ok(at >= 0);
  const body = css.slice(at, css.indexOf("}", at));
  assert.match(body, /background: var\(--bg-active\);/);
  assert.doesNotMatch(body, /box-shadow|border:/);
});

test("the You page asks one question about notifications, with the app's own three answers", () => {
  assert.deepEqual(NOTIFY_PICKS.map(pick => [pick.level, pick.label, pick.icon]), [["all", "All messages", "bell-ring"], ["mentions", "Mentions and DMs", "at-sign"], ["none", "Nothing", "bell-off"]]);
  for (const pick of NOTIFY_PICKS) assert.ok(html.includes(`<option value="${pick.level}">${pick.label}</option>`));
  assert.match(glue, /<div class="sec lbl" id="pt-notify-lbl">Notify me about<\/div>\n<div class="picks" role="radiogroup" aria-labelledby="pt-notify-lbl">\$\{NOTIFY_PICKS\.map\(pick => `<button type="button" class="srow pick" role="radio" aria-checked="false" data-notify="\$\{pick\.level\}">/);
  assert.match(glue, /const level = notifyPrefs\(\)\.level;\n\s+for \(const row of document\.querySelectorAll\("#pt-you \[data-notify\]"\)\) row\.setAttribute\("aria-checked", String\(row\.dataset\.notify === level\)\);/);
  assert.match(glue, /select\.value = row\.dataset\.notify;\n\s+select\.dispatchEvent\(new Event\("change", \{ bubbles: true \}\)\);\n\s+paintYouPage\(\);/);
  assert.match(css, rule(' \\.srow\\.pick\\[aria-checked="true"\\]', "font-weight: 600;"));
  assert.match(css, rule(" \\.srow\\.pick \\.tick", "color: var\\(--accent-lit\\); opacity: 0;"));
  assert.match(css, rule(' \\.srow\\.pick\\[aria-checked="true"\\] \\.tick', "opacity: 1;"));
  assert.match(css, rule(" \\.pt-you \\.set \\.srow", "height: 52px;"));
  assert.doesNotMatch(css, new RegExp(`${slim} \\.srow\\.pick \\{[^}]*font-size`));
});

test("the early build line shows only while the version is below one, and never when the version is unknown", () => {
  assert.equal(earlyBuild("0.1.19"), true);
  assert.equal(earlyBuild("v0.9.0-rc.2"), true);
  assert.equal(earlyBuild("1.0.0"), false);
  assert.equal(earlyBuild("12.4.1"), false);
  assert.equal(earlyBuild(""), false);
  assert.equal(earlyBuild(null), false);
  assert.equal(earlyBuild("Commy"), false);
  assert.match(glue, /<span class="l" id="pt-about-words">Early build<small>Some of this will move\.<\/small><\/span>/);
  assert.match(glue, /\$\("pt-about-words"\)\.classList\.toggle\("hidden", !earlyBuild\(updateStatus && updateStatus\.current\)\);/);
});

test("the You page ends with All settings under a hairline, the star, and Sign out as red words that still ask first", () => {
  assert.match(glue, /<button type="button" class="srow go" id="pt-settings"><i data-i="settings"><\/i><span class="l">All settings<\/span><i data-i="chevron-right" class="chev"><\/i><\/button>/);
  assert.match(glue, /<button type="button" class="out" id="pt-sign-out">Sign out<\/button>/);
  assert.match(glue, /\$\("pt-sign-out"\)\.onclick = \(\) => signOutOfCurrent\(\);/);
  assert.match(read("scripts", "app.js"), /async function signOutOfCurrent\(\) \{[^}]*confirm\(/);
  assert.match(css, rule(" \\.pt-you \\.set \\.srow\\.go", "box-shadow: inset 0 1px 0 var\\(--line\\);"));
  assert.match(css, rule(" \\.pt-you \\.about \\.out", "background: none;[^}]*color: var\\(--danger-text\\);"));
  assert.doesNotMatch(css, new RegExp(`${slim} \\.out \\{`));
  assert.doesNotMatch(glue, /<div class="sec lbl">More<\/div>/);
});

test("the You page keeps the design guide's measures, and the pressed Theme key is the darker one in Day", () => {
  assert.match(css, rule(" \\.pt-page \\.sec", "align-items: center;\\s+height: 32px;\\s+padding: 14px 12px 0;"));
  assert.match(css, rule(" \\.acct \\.avatar \\.presence-dot", "width: 13px; height: 13px; border: 0; box-shadow: 0 0 0 2\\.5px var\\(--bg-dark\\);"));
  assert.match(css, rule(" \\.acct \\.t b", "margin: -\\.2em 0; padding: \\.2em 0;"));
  assert.match(css, rule(" \\.pt-you \\.set \\.seg", "gap: 2px; padding: 4px; border-radius: 999px;"));
  assert.match(css, rule('\\[data-theme="light"\\] \\.seg \\.key\\.is-down', "background: var\\(--pillow-top\\), var\\(--bg-pressed\\); filter: none;"));
  assert.match(css, rule(" \\.seg \\.key\\.is-down \\.led", "box-shadow: 0 0 0 1\\.5px rgb\\(var\\(--dark\\) / \\.5\\);"));
});

test("a server banner is the top of the Home pane, under the glass, and fades out before the chips", () => {
  const head = ' \\.app\\[data-pane="list"\\]\\[data-tab="home"\\]:not\\(\\.dm-mode\\) \\.sidebar-header\\.has-banner:not\\(\\.plain\\)';
  assert.match(css, rule(head, "height: 110px; padding-top: 66px;"));
  assert.match(css, rule(`${head} > \\.sidebar-banner`, "display: block;\\s+position: absolute;\\s+inset: 0 0 auto;\\s+z-index: 0;\\s+height: 132px;\\s+border-radius: var\\(--radius-xl\\) var\\(--radius-xl\\) 0 0;"));
  assert.match(css, rule(`${head} > \\.sidebar-banner`, "mask-image: linear-gradient\\(#000 30%, transparent\\);\\s+pointer-events: none;"));
  assert.match(css, rule(`${head} > \\.sidebar-banner::after`, "display: none;"));
  assert.match(css, rule(' \\.app\\[data-pane="list"\\] \\.sidebar-banner', "display: none;"));
});

test("a channel banner never paints its row in the phone's channel list", () => {
  assert.match(css, new RegExp(`${slim} \\.app\\[data-pane="list"\\] \\.channel\\.has-banner::before,\\n${slim} \\.app\\[data-pane="list"\\] \\.channel\\.has-banner::after \\{ display: none; \\}`));
  assert.match(css, rule(' \\.app\\[data-pane="list"\\] \\.channel\\.has-banner :is\\(\\.hash, \\.channel-label\\)', "text-shadow: none;"));
});

test("a channel banner is the top of the chat pane, and messages are gone before they reach it", () => {
  assert.equal(chatBanner({ view: "server", channel: { id: "c1", banner: "pic@host" } }), "pic@host");
  assert.equal(chatBanner({ view: "server", channel: { id: "c1", banner: null } }), "");
  assert.equal(chatBanner({ view: "server", channel: null }), "");
  assert.equal(chatBanner({ view: "dm", channel: { id: "c1", banner: "pic@host" } }), "");
  assert.match(glue, /const banner = el\("div", "pt-chat-ban"\);\n\s+banner\.id = "pt-chat-ban";\n\s+document\.querySelector\("\.main"\)\.appendChild\(banner\);/);
  assert.match(glue, /const ref = chatBanner\(\{ view: state\.view, channel: state\.currentChannel \}\);\n\s+if \(node\.dataset\.ref === ref\) return;/);
  assert.match(glue, /document\.querySelector\("\.main"\)\.classList\.toggle\("pt-has-ban", Boolean\(ref\)\);/);
  assert.match(glue, /if \(url && node\.dataset\.ref === ref\) node\.style\.backgroundImage = `url\("\$\{url\}"\)`;/);
  const pane = ' \\.app\\[data-pane="chat"\\] > \\.main\\.pt-pane\\.pt-has-ban';
  assert.match(css, rule(`${pane} > \\.pt-chat-ban`, "display: block;\\s+position: absolute;\\s+z-index: -1;\\s+left: 10px;\\s+right: 10px;\\s+top: var\\(--pt-pane-y, 66px\\);\\s+height: 88px;"));
  assert.match(css, rule(`${pane} > \\.pt-chat-ban`, "mask-image: linear-gradient\\(#000 20%, transparent\\);"));
  assert.match(css, rule(`${pane} \\.messages`, "mask-image: linear-gradient\\(180deg, transparent 0, transparent 70px, #000 118px\\);"));
  assert.match(css, rule(" \\.channel-intro-banner", "display: none;"));
});

test("the invite screen is a sheet over the server's wall, with the app's own words and two keys", () => {
  assert.match(glue, /<small>You've been invited to join a server<\/small>\n<b id="pt-invite-name"><\/b>\n<span class="ifrom" id="pt-invite-from"><\/span>/);
  assert.ok(read("scripts", "invite-link.js").includes(`"invite-card-title", "You've been invited to join a server"`));
  assert.match(glue, /<div class="ikeys"><button type="button" class="key lg primary" id="pt-invite-join">Join<\/button><button type="button" class="key lg" id="pt-invite-later">Not now<\/button><\/div>/);
  assert.match(glue, /const invite = studioNode\("div", "pt-invite", inviteMarkup\(\)\);\n\s+invite\.id = "pt-invite";\n\s+document\.body\.appendChild\(invite\);/);
  assert.match(glue, /return studioSlim\(\) && !\$\("modal-backdrop"\)\.classList\.contains\("hidden"\) && !\$\("modal-invite-link"\)\.classList\.contains\("hidden"\);/);
  assert.match(glue, /\$\("pt-invite-name"\)\.textContent = \$\("invite-link-name"\)\.textContent;\n\s+\$\("pt-invite-from"\)\.textContent = \$\("invite-link-host"\)\.textContent;/);
  assert.match(glue, /wall\.style\.backgroundImage = picture;\n\s+wall\.classList\.toggle\("wash", !picture\);/);
  assert.match(css, rule(" \\.pt-invite\\.on", "position: fixed; inset: 0; z-index: 170; display: block;"));
  assert.match(css, rule(" \\.pt-invite \\.iwall\\.wash", "radial-gradient\\(120% 70% at 50% 0%, color-mix\\(in srgb, var\\(--c\\) 58%, var\\(--bg-dark\\)\\), var\\(--bg-dark\\) 78%\\);"));
  assert.match(css, rule(" \\.pt-invite \\.isheet", "bottom: var\\(--hz-safe\\);\\s+display: flex;\\s+flex-direction: column;\\s+max-height: calc\\(100% - env\\(safe-area-inset-top\\) - var\\(--hz-safe\\) - 56px\\);"));
  assert.match(css, rule(" \\.pt-invite \\.ibody", "min-height: 0; overflow-y: auto;"));
  assert.match(css, rule(" \\.pt-invite \\.ikeys", "flex: none; display: grid; gap: 8px; margin-top: 24px;"));
  assert.match(css, rule(" \\.pt-invite \\.sico", "width: 72px; height: 72px; border-radius: 22px; background: var\\(--c\\);"));
});

test("the warning on an invite from another server keeps its words, on a quiet tungsten block that is never red", () => {
  const words = "This invite comes from tide.example, not your account server (commy.example). Anyone can run a Commy server, so only join if you trust the person who sent this link. Your account stays on your own server either way.";
  const parts = inviteWarningParts(words);
  assert.equal(parts.lead, "This invite comes from tide.example, not your account server (commy.example).");
  assert.equal(parts.lead + parts.rest, words);
  assert.deepEqual(inviteWarningParts(""), { lead: "", rest: "" });
  assert.deepEqual(inviteWarningParts("One sentence only."), { lead: "One sentence only.", rest: "" });
  assert.match(glue, /<div class="iwarn hidden" id="pt-invite-warn" role="alert"><span class="iw"><i data-i="triangle-alert"><\/i><\/span>/);
  assert.match(glue, /const parts = inviteWarningParts\(warning\.classList\.contains\("hidden"\) \? "" : warning\.textContent\);\n\s+\$\("pt-invite-warn"\)\.classList\.toggle\("hidden", !parts\.lead\);/);
  assert.match(css, rule(" \\.pt-invite \\.iwarn", "border-radius: 18px; background: color-mix\\(in srgb, var\\(--accent-2\\) 13%, var\\(--bg-active\\)\\);"));
  assert.match(css, rule(" \\.pt-invite \\.iwarn \\.iw", "background: var\\(--accent-2\\); color: var\\(--accent-2-ink\\);"));
  const block = css.slice(css.indexOf(`html[data-look="studio"][data-fit="slim"] .pt-invite .iwarn {`), css.indexOf(`html[data-look="studio"][data-fit="slim"] .pt-invite .ierr`));
  assert.doesNotMatch(block, /danger|--red|--live/);
});

test("leaving an invite: Not now and back close it, and nothing closes it while it is joining", () => {
  assert.equal(layerStep("ptInvite", true, false, { pane: "list" }), "push");
  assert.equal(layerStep("ptInvite", true, false, null), "push");
  assert.equal(layerStep("ptInvite", true, true, { pane: "list", ptInvite: true }), "keep");
  assert.equal(layerStep("ptInvite", false, true, { pane: "list", ptInvite: true }), "back");
  assert.equal(layerStep("ptInvite", false, true, { pane: "list" }), "none");
  assert.equal(layerStep("ptInvite", false, false, { pane: "list", ptInvite: true }), "none");
  assert.match(glue, /\$\("pt-invite-later"\)\.onclick = \(\) => \$\("invite-link-cancel"\)\.click\(\);/);
  assert.match(glue, /if \(\(history\.state && history\.state\.ptInvite\) \|\| !inviteShown\(\)\) return;\n\s+if \(studioPhone\.inviteBusy\) \{\n\s+history\.pushState\(\{ \.\.\.\(history\.state \|\| \{ pane: phonePane\(\) \}\), ptInvite: true \}, ""\);\n\s+return;\n\s+\}\n\s+studioPhone\.inviteStep = false;\n\s+\$\("invite-link-cancel"\)\.click\(\);/);
  assert.doesNotMatch(glue, /\$\("pt-invite-wall"\)\.onclick/);
});

test("the Accounts sheet on the phone has its own back step and closes on the scrim or a swipe down", () => {
  assert.match(glue, /const step = layerStep\("ptAccounts", open, studioPhone\.accountsStep, history\.state\);/);
  assert.match(glue, /if \(\(history\.state && history\.state\.ptAccounts\) \|\| !accountsShown\(\)\) return;\n\s+studioPhone\.accountsStep = false;\n\s+hideModals\(\);/);
  assert.match(glue, /if \(e\.target === e\.currentTarget && accountsShown\(\)\) hideModals\(\);/);
  assert.match(glue, /const STUDIO_SHEET_NODES = "[^"]*#modal-accounts"/);
  assert.match(glue, /else if \(sheet\.id === "modal-accounts"\) hideModals\(\);/);
  assert.match(glue, /\n  wireAccountsSheet\(\);\n/);
});

test("joining from the invite screen says Joining, and a failure shows the app's words in red text above the keys", () => {
  assert.match(glue, /\$\("pt-invite-join"\)\.disabled = busy;\n\s+\$\("pt-invite-later"\)\.disabled = busy;\n\s+\$\("pt-invite-join"\)\.textContent = busy \? "Joining\\u2026" : "Join";/);
  const calls = /const summary = await invoke\("join_server", \{ ticket: invite\.ticket \}\);\n\s+pendingInvite = null;\n\s+hideModals\(\);\n\s+await enterJoinedServer\(summary\);/;
  assert.match(glue, calls);
  assert.match(read("scripts", "invite-link.js"), calls);
  assert.match(glue, /\} catch \(e\) \{\n\s+\$\("pt-invite-error"\)\.textContent = String\(e\);\n\s+\$\("pt-invite-error"\)\.classList\.toggle\("hidden", false\);\n\s+\} finally \{\n\s+studioPhone\.inviteBusy = false;\n\s+paintInviteKeys\(\);/);
  assert.match(css, rule(" \\.pt-invite \\.ierr", "color: var\\(--danger-text\\);"));
});

test("the line under your name on You is the presence key, and it opens a sheet that asks for how long", () => {
  assert.match(glue, /<button type="button" class="presence" id="pt-presence"><span class="pw" id="pt-me-status"><\/span><span class="pend" id="pt-me-until"><\/span><i data-i="chevron-down" class="pchev"><\/i><\/button>/);
  assert.match(glue, /\$\("pt-presence"\)\.onclick = openPresenceSheet;/);
  assert.match(glue, /row\.onclick = \(\) => choice\.presence === "online" \? closeSheets\(\(\) => setOwnPresence\("online", null\)\) : showDurationStep\(choice\.presence\);/);
  assert.match(glue, /await saveOwnProfile\(\{ presence \}\);\n\s+await setStatusExpiry\(\{ presence: until \}\);/);
  assert.match(glue, /if \(until && key\.scrollWidth > key\.parentNode\.clientWidth\) end\.classList\.add\("hidden"\);/);
  assert.match(glue, /const STUDIO_SHEETS = \[[^\]]*"pt-pick-open"/);
  assert.match(css, rule(' \\.acct \\.t \\.presence:not\\(\\[data-presence="online"\\]\\) \\.pw,\\n[^{]*\\.acct \\.t \\.presence \\.pend', "color: var\\(--text-muted\\);"));
  assert.match(css, /\.pt-pick\.pt-sizing \{ overflow: hidden; transition: height 320ms var\(--ease-settle\); \}/);
  assert.match(css, /\.presence-dot\.do_not_disturb::after \{ content: ""; position: absolute; left: 22%; right: 22%;/);
});

test("Switch Account sits under All settings whenever there is an account to switch from", () => {
  assert.match(glue, /<button type="button" class="srow go hidden" id="pt-switch-account"><i data-i="users"><\/i><span class="l">Switch Account<\/span>/);
  assert.match(glue, /\$\("pt-switch-account"\)\.classList\.toggle\("hidden", !accountRows\(account\)\.length\);/);
  assert.match(glue, /\$\("pt-switch-account"\)\.onclick = \(\) => openAccounts\(\);/);
});

test("the pencil on You opens the profile editor, which saves name and status on Save and asks before dropping them", () => {
  assert.match(glue, /\$\("pt-edit"\)\.onclick = e => \{\n\s+e\.stopPropagation\(\);\n\s+openProfileEditor\(\);/);
  assert.match(glue, /return \$\("pt-prof-dn"\)\.value !== saved\.name\n\s+\|\| \$\("pt-prof-status"\)\.value !== saved\.status\n\s+\|\| studioPhone\.profileEmoji !== saved\.emoji\n\s+\|\| studioPhone\.profileClear !== undefined;/);
  assert.match(glue, /\$\("pt-prof-save"\)\.disabled = !profileDirty\(\);/);
  assert.match(glue, /function leaveProfileEditor\(\) \{\n\s+if \(!profileDirty\(\)\) return closeProfileEditor\(\);\n\s+openPickSheet\("Save your changes\?"/);
  assert.match(glue, /if \(studioPhone\.profileClear !== undefined\) await setStatusExpiry\(\{ status: studioPhone\.profileClear \}\);/);
  assert.match(glue, /if \(profileOpen\(\)\) return edgeBackStarts\(touch\.clientX\) && !sheetOpen\(\) \? \{ kind: "back", node: \$\("pt-profile"\), profile: true \} : null;/);
  assert.ok(glue.includes('${field("Display name", "pt-prof-dn", 64)}') && glue.includes('${field("Pronouns", "pt-prof-pro", 24)}'));
  assert.match(glue, /<input type="text" id="pt-prof-status" maxlength="64"/);
  assert.match(glue, /<textarea id="pt-prof-about" maxlength="190"/);
  assert.match(css, rule(" \\.pt-profile \\.eban", "height: 124px; margin: 0 -16px;"));
  assert.match(css, rule(" \\.pt-profile \\.in", "height: 52px; padding: 0 5px 0 18px;[^}]*box-shadow: var\\(--slot\\);"));
  assert.match(css, rule(" \\.pt-profile \\.looks", "grid-template-columns: repeat\\(3, minmax\\(0, 1fr\\)\\); gap: 8px;"));
  assert.doesNotMatch(css, new RegExp(`${slim} \\.set \\{`));
});

test("tapping a face or a name in chat opens that person's card as a sheet with its own back step", () => {
  assert.deepEqual(faceTapAuthor({ author_id: "u1", author_name: "nova" }), { id: "u1", name: "nova" });
  assert.deepEqual(faceTapAuthor({ author_id: "u1" }), { id: "u1", name: "" });
  assert.equal(faceTapAuthor({ author_name: "nova" }), null);
  assert.equal(faceTapAuthor(null), null);
  assert.match(glue, /const hit = e\.target\.closest\("\.msg > \.avatar:not\(\.spacer\), \.msg-head > \.msg-author"\);/);
  assert.match(glue, /openMemberCard\(fullMember\(author\), hit\);/);
  assert.match(glue, /const card = layerStep\("ptCard", open, studioPhone\.cardStep, entry\);/);
  assert.match(glue, /const more = layerStep\("ptCardMenu", menu, studioPhone\.cardMenuStep, entry\);/);
  assert.match(glue, /const steps = card === "back" \? \(entry\.ptCardMenu \? 2 : 1\) : more === "back" \? 1 : 0;/);
  assert.match(glue, /watch\(\$\("fp-menu"\), \{ attributes: true, attributeFilter: \["class"\] \}, syncCardSheet\);/);
  assert.match(glue, /const STUDIO_SHEET_NODES = "[^"]*\.member-card, /);
  assert.match(css, rule(" \\.app\\.pt-card-open > \\.pt-scrim", "background: var\\(--scrim\\);"));
  assert.match(css, rule(" \\.member-card", "inset: auto 0 var\\(--keyboard-height, 0px\\) 0 !important;"));
  assert.match(css, rule(" \\.member-card", "max-height: calc\\(var\\(--app-height, 100dvh\\) - env\\(safe-area-inset-top\\) - 56px\\);"));
  assert.match(css, rule(" \\.member-card > \\.mc-body", "overflow-y: auto;"));
});

test("the card sheet wears the design guide's phone sizes on top of the shared card", () => {
  assert.match(css, rule(" \\.member-card \\.mc-banner", "height: 128px;"));
  assert.match(css, rule(" \\.member-card \\.mc-head", "margin-top: -54px;"));
  assert.match(css, rule(" \\.member-card \\.mc-head \\.avatar", "width: 88px; height: 88px;"));
  assert.match(css, rule(" \\.member-card \\.mc-actions \\.key", "height: 44px; padding: 0 18px 0 15px; font-size: 15px;"));
  assert.match(css, rule(" \\.member-card > \\.mc-more", "top: 14px; right: 14px; width: 44px; height: 44px;"));
  assert.match(css, rule(" \\.member-card \\.mc-name", "font-size: 25px; line-height: 28px;"));
  assert.match(css, rule(" \\.member-card \\.mc-about", "font: 400 16px/23px var\\(--font\\);"));
  assert.match(css, rule(" \\.member-card \\.mc-role", "height: 30px;"));
  assert.match(css, rule(" \\.member-card \\.mc-field", "height: 52px;"));
  assert.match(css, rule(" \\.member-card\\.slab::after", "inset: 9px auto auto 50%; opacity: 1; mix-blend-mode: normal;"));
});

test("Edit profile on your own card opens the phone's profile editor once the card has stepped back", () => {
  assert.match(glue, /\$\("mc-actions"\)\.addEventListener\("click", editFromCard, true\);/);
  assert.match(glue, /if \(!studioSlim\(\) \|\| !e\.target\.closest\("#mc-edit"\)\) return;/);
  assert.match(glue, /window\.addEventListener\("popstate", \(\) => openProfileEditor\(\), \{ once: true \}\);\n  closeFloating\(\);/);
  assert.match(glue, /\$\("pt-profile"\)\.style\.setProperty\("--pc", personTint\(user\.id\)\);/);
});

test("invite cards in a phone chat follow the same design guide rules as on desktop, answering to their own width", () => {
  assert.match(css, rule(" \\.invite-card,\\n[^{]*\\.voice-invite", "container: icard / inline-size;"));
  assert.match(css, rule(" \\.invite-card,\\n[^{]*\\.voice-invite", "grid-template-columns: 44px minmax\\(0, 1fr\\) auto;"));
  assert.match(css, rule(" \\.invite-card,\\n[^{]*\\.voice-invite", "border-radius: 24px;"));
  assert.match(css, new RegExp(`@container icard \\(max-width: 339px\\) \\{\\n  ${slim} \\.invite-card-text,`));
  assert.match(css, new RegExp(`@container icard \\(max-width: 259px\\) \\{\\n  ${slim} \\.invite-card-mark,`));
  assert.match(css, rule(" \\.invite-card > button,\\n[^{]*", "min-width: 84px;\\s+height: 40px;"));
});

test("mutual servers on the phone's card use 36 px tiles lined up with the card's words, and the field stays pinned under the scrolling middle", () => {
  assert.match(css, rule(" \\.member-card \\.prof-servers", "margin: 6px 14px 0;"));
  assert.match(css, rule(" \\.member-card \\.prof-server", "min-height: 52px;"));
  assert.match(css, rule(" \\.member-card \\.prof-tile", "width: 36px; height: 36px;"));
  assert.match(css, rule(" \\.member-card \\.prof-name", "font-size: 16px; font-weight: 500;"));
  assert.match(css, / \.member-card > \.mc-body\.pt-cut \{\n  -webkit-mask-image: linear-gradient\(180deg, #000 calc\(100% - 14px\), transparent\);\n  mask-image: linear-gradient\(180deg, #000 calc\(100% - 14px\), transparent\);/);
  assert.match(glue, /body\.classList\.toggle\("pt-cut", studioSlim\(\) && body\.scrollHeight - body\.scrollTop - body\.clientHeight > 1\);/);
  assert.match(glue, /const home = studioSlim\(\) \? \$\("member-card"\) : \$\("member-card"\)\.querySelector\("\.mc-body"\);\n  if \(field\.parentNode !== home\) home\.appendChild\(field\);/);
});

test("holding a message lifts a copy of it above the guide's bottom sheet, with the reactions across the sheet's top edge", () => {
  const rule = selector => {
    const start = css.indexOf(`html[data-look="studio"][data-fit="slim"] ${selector} {`);
    assert.ok(start >= 0, selector);
    return css.slice(start, css.indexOf("}", start));
  };
  const sheet = rule(".app.pt-hold-open > .pt-hold");
  for (const line of ["left: 0;", "right: 0;", "bottom: 0;", "padding: 50px 14px max(14px, calc(var(--hz-safe) - 3px));", "border-radius: var(--radius-sheet) var(--radius-sheet) 0 0;", "var(--sheet-bg);", "box-shadow: inset 0 0 0 1px var(--rim-soft), 0 -24px 40px -10px rgb(var(--dark) / .8);"]) assert.ok(sheet.includes(line), line);
  const rx = rule(".pt-hold .rx");
  for (const line of ["left: 16px;", "right: 16px;", "top: -34px;", "height: 66px;", "padding: 0 9px;", "border-radius: 999px;", "var(--bg-mid);", "box-shadow: inset 0 0 0 1px var(--rim-soft), var(--shadow-3);"]) assert.ok(rx.includes(line), line);
  const copy = rule(".app > .pt-hold > .msg.pt-hold-msg");
  for (const line of ["left: 14px;", "right: 14px;", "bottom: calc(100% + 15px);", "padding: 14px 14px 16px;", "border-radius: 28px;", "pointer-events: none;"]) assert.ok(copy.includes(line), line);
  assert.ok(css.includes('html[data-look="studio"][data-fit="slim"] .msg.pt-held { visibility: hidden; }'));
  assert.ok(css.includes('html[data-look="studio"][data-fit="slim"] .app:is(.pt-search-open, .pt-hold-open) > .pt-scrim { background: var(--scrim); }'));
  assert.match(glue, /const copy = heldCopy\(msg\);\n  \$\("pt-hold"\)\.prepend\(copy\);/);
  assert.match(glue, /for \(const copy of document\.querySelectorAll\("\.pt-hold-msg"\)\) copy\.remove\(\);\n  \$\("pt-hold"\)\?\.classList\.remove\("pt-tmore"\);\n\}/);
  assert.match(glue, /copy\.inert = true;/);
});

test("an open thread on the phone is a full screen like the chat, with its own back step and a More sheet", () => {
  const P = 'html[data-look="studio"][data-fit="slim"]';
  const rule = selector => {
    const start = css.indexOf(`${P} ${selector} {`);
    assert.ok(start >= 0, selector);
    return css.slice(start, css.indexOf("}", start));
  };
  assert.ok(rule('.app[data-pane="chat"].thread-open > .thread-panel').includes("grid-template-rows: 60px minmax(0, 1fr) auto;"));
  assert.ok(rule('.app[data-pane="chat"].thread-open > .thread-panel::before').includes("margin: 6px 10px 11px;"));
  assert.ok(rule(".thread-panel .thread-head").includes("grid-template-columns: 44px minmax(0, 1fr) auto;"));
  assert.ok(rule(".thread-panel :is(.thread-title, .thread-title-input)").includes("font: 700 22px/22px var(--font-display);"));
  assert.ok(css.includes(`${P} .thread-panel .thread-status::after { content: " \\00b7  " attr(data-where); }`));
  assert.ok(css.includes(`${P} .thread-panel .thread-divider:last-child { display: none; }`));
  assert.ok(css.includes(`${P} .thread-panel :is(.thread-head-icon, .thread-head > .icon-btn) { display: none; }`));
  assert.ok(rule(".thread-panel .thread-composer").includes("margin: 0 16px calc(var(--hz-line) + 7px);"));
  assert.ok(css.includes(`${P} .app.pt-hold-open > .pt-hold.pt-tmore { padding: 30px 14px calc(var(--hz-safe) + 8px); }`));
  assert.ok(css.includes(`${P} .pt-hold .pt-sheet-rows > .arow:first-child { margin-bottom: 5px; }`));
  assert.equal(sheetRow("Rename Thread").label, "Rename thread");
  assert.equal(sheetRow("Unlock Thread").label, "Unlock thread");
  assert.equal(sheetRow("Archive Thread").icon, "archive");
  assert.match(glue, /const step = layerStep\("ptThread", open, studioPhone\.threadStep, entry\);/);
  assert.match(glue, /if \(!\(history\.state && history\.state\.ptThread\) && studioPhone\.threadStep\) \{\n      studioPhone\.threadStep = false;\n      closeThread\(\);/);
  assert.match(glue, /return \{ kind: "back", node: \$\("thread-panel"\), thread: true \};/);
  assert.match(glue, /send\.onclick = \(\) => sendThreadReply\(\);/);
  assert.match(glue, /more\.classList\.toggle\("hidden", !threadMoreRows\(\)\.length\);/);
});
