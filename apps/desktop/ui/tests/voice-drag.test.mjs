import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { VOICE_DRAG, canDragVoiceMember, voiceDragStep, voiceDropRoom, voiceEdgeScroll, voiceMoveLanded, voiceMoveStuckText } = require("../scripts/voice-drag.js");

const server = {
  owner: "owner",
  can_move: true,
  channels: [
    { id: "general", kind: "voice" },
    { id: "afk", kind: "voice" },
    { id: "stage", kind: "stage" },
    { id: "chat", kind: "text" },
    { id: "hidden", kind: "voice", can_view: false },
  ],
};

test("only people who may move members drag someone else, and never the owner", () => {
  assert.ok(canDragVoiceMember(server, "friend", "me"));
  assert.ok(!canDragVoiceMember(server, "owner", "me"));
  assert.ok(!canDragVoiceMember({ ...server, can_move: false }, "friend", "me"));
  assert.ok(!canDragVoiceMember(server, null, "me"));
  assert.ok(!canDragVoiceMember(null, "friend", "me"));
});

test("a mouse drags after a small move, a finger only after holding still", () => {
  assert.equal(voiceDragStep("mouse", 2, 2, 0), "wait");
  assert.equal(voiceDragStep("mouse", VOICE_DRAG.mouseSlop + 1, 0, 0), "drag");
  assert.equal(voiceDragStep("pen", 0, VOICE_DRAG.mouseSlop + 1, 0), "drag");
  assert.equal(voiceDragStep("touch", 3, 3, VOICE_DRAG.touchHoldMs - 1), "wait");
  assert.equal(voiceDragStep("touch", 3, 3, VOICE_DRAG.touchHoldMs), "drag");
  assert.equal(voiceDragStep("touch", VOICE_DRAG.touchSlop + 1, 0, 100), "cancel", "a swipe scrolls the list");
});

test("people drop into another voice or stage channel they can see", () => {
  assert.equal(voiceDropRoom(server, "general", "afk").id, "afk");
  assert.equal(voiceDropRoom(server, "general", "stage").id, "stage");
  assert.equal(voiceDropRoom(server, "general", "general"), null, "the room they are in");
  assert.equal(voiceDropRoom(server, "general", "chat"), null);
  assert.equal(voiceDropRoom(server, "general", "hidden"), null);
  assert.equal(voiceDropRoom(server, "general", "gone"), null);
  assert.equal(voiceDropRoom(server, "general", null), null);
  assert.equal(voiceDropRoom(null, "general", "afk"), null);
});

test("dragging near the top or bottom of the channel list scrolls it", () => {
  assert.equal(voiceEdgeScroll(105, 100, 600), -VOICE_DRAG.edgeStep);
  assert.equal(voiceEdgeScroll(595, 100, 600), VOICE_DRAG.edgeStep);
  assert.equal(voiceEdgeScroll(350, 100, 600), 0);
});

test("a move has landed once the person shows in the room, or out of voice after a disconnect", () => {
  const view = { members: [{ id: "friend", voice_channel: "afk" }, { id: "left", voice_channel: null }] };
  assert.ok(voiceMoveLanded(view, "friend", "afk"));
  assert.ok(!voiceMoveLanded(view, "friend", "general"));
  assert.ok(voiceMoveLanded(view, "left", null));
  assert.ok(!voiceMoveLanded(view, "friend", null));
  assert.ok(voiceMoveLanded(view, "gone", "afk"), "someone who left the server is not waited on");
  assert.match(voiceMoveStuckText("Sam", "AFK"), /Sam has not switched to AFK yet/);
  assert.match(voiceMoveStuckText("Sam", null), /Sam is still in voice/);
});

test("everyone may drag themselves to another room, the owner too", () => {
  assert.ok(canDragVoiceMember(server, "me", "me"));
  assert.ok(canDragVoiceMember({ ...server, can_move: false }, "me", "me"));
  assert.ok(canDragVoiceMember({ ...server, owner: "me", can_move: false }, "me", "me"));
});
