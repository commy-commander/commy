import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { meterPercent, sensitivityLabel, meterState, inputModeCopy, talkKeyTitle, voiceInputState, TALK_RELEASE_OPTIONS } = require("../scripts/voice-input.js");

test("the meter maps decibels full scale onto a bar", () => {
  assert.equal(meterPercent(-100), 0);
  assert.equal(meterPercent(0), 100);
  assert.equal(meterPercent(-50), 50);
  assert.equal(meterPercent(NaN), 0);
  assert.equal(meterPercent(5), 100);
});

test("the sensitivity label reads Automatic or the threshold", () => {
  assert.equal(sensitivityLabel(true, -20), "Automatic");
  assert.equal(sensitivityLabel(false, -42.6), "-43 dB");
  assert.equal(sensitivityLabel(false, undefined), "-45 dB");
});

test("the meter colours by the gate that is in force", () => {
  assert.equal(meterState(-30, false, -40, true), "open");
  assert.equal(meterState(-50, false, -40, true), "quiet");
  assert.equal(meterState(-30, true, -40, true), "open");
  assert.equal(meterState(-70, true, -40, true), "quiet");
  assert.equal(meterState(-30, false, -40, false), "idle");
});

test("settings read with defaults for older saves", () => {
  assert.deepEqual(voiceInputState({}), { mode: "voice_activity", talkKey: "Ctrl+Shift+Space", releaseMs: 200, muteKey: "", deafenKey: "", auto: true, thresholdDb: -45 });
  const s = voiceInputState({ input_mode: "push_to_talk", talk_key: "Alt+Q", talk_release_ms: 500, mute_hotkey: "Ctrl+M", deafen_hotkey: "", auto_sensitivity: false, sensitivity_db: -30 });
  assert.deepEqual(s, { mode: "push_to_talk", talkKey: "Alt+Q", releaseMs: 500, muteKey: "Ctrl+M", deafenKey: "", auto: false, thresholdDb: -30 });
  assert.equal(TALK_RELEASE_OPTIONS.some(([ms]) => ms === 200), true);
});

test("copy follows the mode and the talk key", () => {
  assert.equal(inputModeCopy("push_to_talk").title, "Push to Talk");
  assert.equal(inputModeCopy("voice_activity").title, "Voice Activity");
  assert.equal(talkKeyTitle("push_to_talk", "Alt+Q", false), "Hold Alt+Q to talk");
  assert.equal(talkKeyTitle("push_to_talk", "Alt+Q", true), "Talking (Alt+Q)");
  assert.equal(talkKeyTitle("voice_activity", "Alt+Q", true), "");
});
