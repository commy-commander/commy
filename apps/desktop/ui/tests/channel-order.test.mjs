import "./i18n-global.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { CATEGORY_BASE_POSITION, reorderKeys, looseStayFirst, categoryMoves, sectionChannelMoves, dropLandsAfter, sectionStep, channelStep, stepMoves } = require("../scripts/channel-order.js");

const keys = ["text", "voice", "a", "b", "c"];

test("a group lands before or after the one it is dropped on", () => {
  assert.deepEqual(reorderKeys(keys, "c", "a", false), ["text", "voice", "c", "a", "b"]);
  assert.deepEqual(reorderKeys(keys, "a", "c", true), ["text", "voice", "b", "c", "a"]);
  assert.deepEqual(reorderKeys(keys, "voice", "text", false), ["voice", "text", "a", "b", "c"]);
  assert.deepEqual(reorderKeys(keys, "text", "c", true), ["voice", "a", "b", "c", "text"]);
});

test("without a known target it goes to the top or the bottom", () => {
  assert.deepEqual(reorderKeys(keys, "b", null, false), ["b", "text", "voice", "a", "c"]);
  assert.deepEqual(reorderKeys(keys, "a", "gone", true), ["text", "voice", "b", "c", "a"]);
});

test("a drop that changes nothing gives nothing", () => {
  assert.equal(reorderKeys(keys, "a", "a", false), null);
  assert.equal(reorderKeys(keys, "a", "b", false), null);
  assert.equal(reorderKeys(keys, "b", "a", true), null);
  assert.equal(reorderKeys(keys, "gone", "a", false), null);
});

test("the built-in groups only need to become categories once they leave the top", () => {
  assert.equal(looseStayFirst(["text", "voice", "c", "a", "b"], ["text", "voice"]), true);
  assert.equal(looseStayFirst(["voice", "text", "a", "b", "c"], ["text", "voice"]), false);
  assert.equal(looseStayFirst(["text", "a", "voice", "b", "c"], ["text", "voice"]), false);
  assert.equal(looseStayFirst(["a", "b"], []), true);
});

test("moves give categories fresh top level positions and channels their group", () => {
  assert.deepEqual(categoryMoves(["b", "a"], k => `id-${k}`), [
    { channel: "id-b", parent: null, position: CATEGORY_BASE_POSITION },
    { channel: "id-a", parent: null, position: CATEGORY_BASE_POSITION + 1 },
  ]);
  assert.deepEqual(sectionChannelMoves([{ id: "x" }, { id: "y" }], "cat"), [
    { channel: "x", parent: "cat", position: 0 },
    { channel: "y", parent: "cat", position: 1 },
  ]);
});

test("the lower half of a header means after it", () => {
  const rect = { top: 100, height: 20 };
  assert.equal(dropLandsAfter(105, rect), false);
  assert.equal(dropLandsAfter(115, rect), true);
});

test("a group steps one place up or down and stops at either end", () => {
  assert.deepEqual(sectionStep(keys, "b", -1), ["text", "voice", "b", "a", "c"]);
  assert.deepEqual(sectionStep(keys, "b", 1), ["text", "voice", "a", "c", "b"]);
  assert.deepEqual(sectionStep(keys, "text", 1), ["voice", "text", "a", "b", "c"]);
  assert.equal(sectionStep(keys, "text", -1), null);
  assert.equal(sectionStep(keys, "c", 1), null);
  assert.equal(sectionStep(keys, "gone", 1), null);
});

const voiceKind = kind => kind === "voice" || kind === "stage";
const rules = {
  takes: (section, channel) => Boolean(section.category) || (section.kind === "voice") === voiceKind(channel.kind),
  voice: voiceKind,
};
const tapSections = [
  { key: "text", kind: "text", parent: null, channels: [{ id: "welcome", kind: "text" }, { id: "rules", kind: "announcement" }] },
  { key: "voice", kind: "voice", parent: null, channels: [{ id: "lounge", kind: "voice" }] },
  { key: "games", kind: "text", parent: "games", category: { id: "games" }, channels: [{ id: "clips", kind: "text" }, { id: "maps", kind: "forum" }, { id: "party", kind: "voice" }, { id: "stage", kind: "stage" }] },
  { key: "staff", kind: "text", parent: "staff", category: { id: "staff" }, channels: [{ id: "mods", kind: "text" }, { id: "huddle", kind: "voice" }] },
  { key: "afk", kind: "voice", parent: null, fixed: true, channels: [{ id: "afk", kind: "voice" }] },
];

test("a channel steps past its neighbour inside its own group", () => {
  assert.deepEqual(channelStep(tapSections, "rules", -1, rules), { parent: null, ids: ["rules", "welcome"] });
  assert.deepEqual(channelStep(tapSections, "clips", 1, rules), { parent: "games", ids: ["maps", "clips"] });
  assert.deepEqual(channelStep(tapSections, "stage", -1, rules), { parent: "games", ids: ["stage", "party"] });
});

test("text and voice channels keep their own runs, so a step only passes the same sort", () => {
  assert.deepEqual(channelStep(tapSections, "maps", 1, rules), { parent: "staff", ids: ["maps", "mods"] });
  assert.deepEqual(channelStep(tapSections, "party", -1, rules), { parent: null, ids: ["lounge", "party"] });
});

test("at the edge of a group a channel steps into the next group that can hold it", () => {
  assert.deepEqual(channelStep(tapSections, "stage", 1, rules), { parent: "staff", ids: ["stage", "huddle"] });
  assert.deepEqual(channelStep(tapSections, "clips", -1, rules), { parent: null, ids: ["welcome", "rules", "clips"] });
  assert.deepEqual(channelStep(tapSections, "rules", 1, rules), { parent: "games", ids: ["rules", "clips", "maps"] });
  assert.equal(channelStep(tapSections, "lounge", -1, rules), null);
  assert.deepEqual(channelStep(tapSections, "lounge", 1, rules), { parent: "games", ids: ["lounge", "party", "stage"] });
});

test("nothing steps into or out of the fixed AFK group", () => {
  assert.equal(channelStep(tapSections, "afk", -1, rules), null);
  assert.equal(channelStep(tapSections, "welcome", -1, rules), null);
  assert.equal(channelStep(tapSections, "huddle", 1, rules), null);
});

test("a step becomes one move per channel in the group it lands in", () => {
  assert.deepEqual(stepMoves({ parent: "games", ids: ["a", "b"] }), [
    { channel: "a", parent: "games", position: 0 },
    { channel: "b", parent: "games", position: 1 },
  ]);
});
