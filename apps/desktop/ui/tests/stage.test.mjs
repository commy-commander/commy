import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  STAGE_KIND, isVoiceKind, isStageChannel, isStageSpeaker, stageRole, stageRoster, stageSections, handState,
  stageChannelOf, stageMenuAction, stageCreateHint,
} = require("../scripts/stage.js");

const stage = (extra = {}) => ({
  id: "s1",
  name: "Town Hall",
  kind: STAGE_KIND,
  voice_users: ["nova", "ash", "pip", "lumen", "dot"],
  voice_user_ids: ["m2", "m6", "m7", "m8", "m9"],
  voice_since: [100, 200, 300, 400, 500],
  voice_muted: [false, true, false, false, false],
  voice_deafened: [false, false, false, true, false],
  speakers: ["m2", "m6"],
  hands: ["m9", "m7"],
  ...extra,
});

test("stage channels count as voice channels, text ones do not", () => {
  assert.ok(isVoiceKind("voice"));
  assert.ok(isVoiceKind("stage"));
  assert.ok(!isVoiceKind("text"));
  assert.ok(!isVoiceKind("announcement"));
  assert.ok(!isVoiceKind(undefined));
  assert.ok(isStageChannel(stage()));
  assert.ok(!isStageChannel({ kind: "voice" }));
  assert.ok(!isStageChannel(null));
});

test("speakers are the listed ones, everyone else in the room is audience", () => {
  const channel = stage();
  assert.ok(isStageSpeaker(channel, "m2"));
  assert.ok(!isStageSpeaker(channel, "m7"));
  assert.ok(!isStageSpeaker(channel, null));
  assert.ok(!isStageSpeaker({ kind: "voice", speakers: ["m2"] }, "m2"), "voice channels have no speakers list");
  assert.equal(stageRole(channel, "m2"), "speaker");
  assert.equal(stageRole(channel, "m7"), "audience");
  assert.equal(stageRole(channel, "me", true), "speaker", "hosts always have the floor");
  assert.equal(stageRole({ kind: "voice" }, "m2"), null);
});

test("the roster keeps each row's own flags and marks raised hands", () => {
  const roster = stageRoster(stage());
  assert.equal(roster.length, 5);
  assert.deepEqual(roster[1], { index: 1, id: "m6", name: "ash", since: 200, muted: true, deafened: false, hand: false });
  assert.equal(roster[2].hand, true);
  assert.equal(roster[3].deafened, true);
  assert.deepEqual(stageRoster({ kind: "voice", voice_users: ["x"] })[0], { index: 0, id: null, name: "x", since: null, muted: false, deafened: false, hand: false });
});

test("sections split speakers from audience and float raised hands to the top in raise order", () => {
  const { speakers, audience } = stageSections(stage());
  assert.deepEqual(speakers.map(e => e.id), ["m2", "m6"]);
  assert.deepEqual(audience.map(e => e.id), ["m9", "m7", "m8"]);
  assert.deepEqual(audience.map(e => e.hand), [true, true, false]);
  const noHands = stageSections(stage({ hands: [] }));
  assert.deepEqual(noHands.audience.map(e => e.id), ["m7", "m8", "m9"], "roster order without hands");
  const voice = stageSections({ kind: "voice", voice_users: ["a", "b"], voice_user_ids: ["1", "2"] });
  assert.equal(voice.speakers.length, 2);
  assert.equal(voice.audience.length, 0);
});

test("the hand button reads differently for audience, raised hands and speakers", () => {
  const channel = stage();
  const listening = handState(channel, "m8");
  assert.equal(listening.role, "audience");
  assert.equal(listening.raised, false);
  assert.equal(listening.text, "In the audience");
  assert.match(listening.detail, /only speakers are heard/);
  assert.equal(listening.label, "Raise hand");
  const asking = handState(channel, "m7");
  assert.equal(asking.raised, true);
  assert.equal(asking.label, "Lower hand");
  const talking = handState(channel, "m2");
  assert.equal(talking.role, "speaker");
  assert.equal(talking.text, "On the stage");
  assert.equal(talking.detail, "You are a speaker here");
  assert.equal(handState(channel, "me", true).role, "speaker");
  assert.equal(handState({ kind: "voice" }, "m2"), null);
});

test("moderators get an invite or a move for members sitting in a stage", () => {
  const channel = stage();
  const server = { channels: [{ id: "v", kind: "voice", voice_user_ids: ["m1"] }, channel] };
  assert.equal(stageChannelOf(server, "m7"), channel);
  assert.equal(stageChannelOf(server, "m1"), null);
  assert.equal(stageChannelOf(server, null), null);
  assert.deepEqual(stageMenuAction(channel, "m7", true), { speaker: true, label: "Invite to speak", done: "invited to speak" });
  assert.deepEqual(stageMenuAction(channel, "m2", true), { speaker: false, label: "Move to audience", done: "moved to the audience" });
  assert.equal(stageMenuAction(channel, "m7", false), null, "plain members see nothing");
  assert.equal(stageMenuAction(channel, "m1", true), null, "not in the room");
  assert.equal(stageMenuAction({ kind: "voice", voice_user_ids: ["m7"] }, "m7", true), null);
});

test("the create dialog gets a stage placeholder and icon", () => {
  assert.deepEqual(stageCreateHint("stage"), { placeholder: "Town Hall", icon: "stage" });
  assert.equal(stageCreateHint("voice"), null);
});
