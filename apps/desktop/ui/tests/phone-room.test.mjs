import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { roomMemberActions, roomMemberActionWords } = require("../scripts/phone-touch.js");
const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const glue = read("scripts", "studio-phone.js");
const menu = read("scripts", "phone-room-menu.js");
const app = read("scripts", "app.js");
const html = read("index.html");

const lounge = { id: "lounge", kind: "voice" };
const quiet = { id: "quiet", kind: "voice" };
const stage = { id: "stage", kind: "stage" };
const secret = { id: "secret", kind: "voice", can_view: false };
const general = { id: "general", kind: "text" };
const server = (can = {}) => ({
  owner: "boss",
  members: [{ id: "me" }, { id: "ferris" }, { id: "boss" }],
  channels: [general, lounge, quiet, stage, secret],
  ...can,
});
const kinds = actions => actions.map(action => action.kind);

test("holding someone in the room offers only what your permissions allow", () => {
  const where = { targetId: "ferris", myId: "me", roomId: "lounge" };
  assert.deepEqual(roomMemberActions(server(), where), []);
  assert.deepEqual(kinds(roomMemberActions(server({ can_move: true }), where)), ["move", "disconnect"]);
  assert.deepEqual(kinds(roomMemberActions(server({ can_kick: true }), where)), ["kick"]);
  assert.deepEqual(kinds(roomMemberActions(server({ can_ban: true }), where)), ["ban"]);
  assert.deepEqual(kinds(roomMemberActions(server({ can_move: true, can_kick: true, can_ban: true }), where)), ["move", "disconnect", "kick", "ban"]);
});

test("move lists the other voice and stage rooms you can see, never the one they are in", () => {
  const [move] = roomMemberActions(server({ can_move: true }), { targetId: "ferris", myId: "me", roomId: "lounge" });
  assert.deepEqual(move.rooms.map(room => room.id), ["quiet", "stage"]);
  const alone = { ...server({ can_move: true }), channels: [general, lounge] };
  assert.deepEqual(kinds(roomMemberActions(alone, { targetId: "ferris", myId: "me", roomId: "lounge" })), ["disconnect"]);
});

test("no menu for yourself, the owner, someone outside the server, or a direct call", () => {
  const all = server({ can_move: true, can_kick: true, can_ban: true });
  assert.deepEqual(roomMemberActions(all, { targetId: "me", myId: "me", roomId: "lounge" }), []);
  assert.deepEqual(roomMemberActions(all, { targetId: "boss", myId: "me", roomId: "lounge" }), []);
  assert.deepEqual(roomMemberActions(all, { targetId: "stranger", myId: "me", roomId: "lounge" }), []);
  assert.deepEqual(roomMemberActions(null, { targetId: "ferris", myId: "me", roomId: "lounge" }), []);
});

test("the menu words match the desktop actions", () => {
  assert.equal(roomMemberActionWords("move", "ferris"), "Move to another room");
  assert.equal(roomMemberActionWords("disconnect", "ferris"), "Disconnect from voice");
  assert.equal(roomMemberActionWords("kick", "ferris"), "Kick ferris");
  assert.equal(roomMemberActionWords("ban", "ferris"), "Ban ferris");
});

test("the Live tab and a voice channel open the same room", () => {
  assert.match(glue, /function openLiveStream\(stream\) \{\n  return openLiveRoom\(stream\.room, stream\);\n\}/);
  assert.match(glue, /if \(row\) joinRoomRow\(row, stream\);/);
  assert.match(glue, /if \(row\.classList\.contains\("connected"\)\) openSheet\("pt-voice-open"\);\n    else joinRoomRow\(row\);/);
  assert.match(glue, /  queueRoomPaint\(\);\n  openWantedRoom\(\);\n\}/);
  assert.match(glue, /openSheet\("pt-voice-open"\);\n  if \(wanted\.stream && !wanted\.stream\.mine\) watchScreen\(wanted\.stream\.userId, wanted\.stream\.name\);/);
  assert.equal(glue.includes('await setPhoneTab("home");\n  const row'), false);
});

test("the room menu reuses the desktop moderation steps for the room's own server", () => {
  assert.match(menu, /moveVoiceMember\(member, null, server\)/);
  assert.match(menu, /moveVoiceMember\(member, room, server\)/);
  assert.match(menu, /confirmKick\(member, server\)/);
  assert.match(menu, /confirmBan\(member, server\)/);
  assert.match(app, /async function moveVoiceMember\(member, channel, server = state\.currentServer\) \{/);
  assert.match(app, /function confirmKick\(member, server = state\.currentServer\) \{/);
  assert.match(app, /function confirmBan\(member, server = state\.currentServer\) \{/);
  assert.match(menu, /if \(!openRoomMemberMenu\(roomFaceMember\(face\)\)\) return;\n    e\.preventDefault\(\);/);
  assert.match(html, /<div class="pop-menu ctx-menu hidden" id="room-member-menu" role="menu"><\/div>/);
  assert.match(html, /<script src="scripts\/phone-room-menu\.js"><\/script>/);
  assert.equal(app.match(/"member-menu", "room-member-menu"/g).length, 2);
  assert.equal(/\/\/|\/\*/.test(menu), false);
});
