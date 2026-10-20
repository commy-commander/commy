import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { shareAudioChoice, shareAudioSaveValue, streamVolumePercent, streamVolumeGain, streamVolumeTitle, liveToastText, liveSourceText } = require("../scripts/stream-audio.js");

test("share audio defaults on where it is supported and stays off where it is not", () => {
  assert.equal(shareAudioChoice(null, true), true);
  assert.equal(shareAudioChoice("on", true), true);
  assert.equal(shareAudioChoice("off", true), false);
  assert.equal(shareAudioChoice(null, false), false);
  assert.equal(shareAudioChoice("on", false), false);
  assert.equal(shareAudioSaveValue(true), "on");
  assert.equal(shareAudioSaveValue(false), "off");
});

test("the stream volume slider reads the saved gain with a full default", () => {
  assert.equal(streamVolumePercent({}), 100);
  assert.equal(streamVolumePercent(null), 100);
  assert.equal(streamVolumePercent({ stream_volume: 0.5 }), 50);
  assert.equal(streamVolumePercent({ stream_volume: 7 }), 200);
  assert.equal(streamVolumePercent({ stream_volume: -1 }), 0);
});

test("slider positions become gains the backend accepts", () => {
  assert.equal(streamVolumeGain(100), 1);
  assert.equal(streamVolumeGain(0), 0);
  assert.equal(streamVolumeGain(250), 2);
  assert.equal(streamVolumeGain(NaN), 1);
});

test("copy names the volume or the muted stream", () => {
  assert.equal(streamVolumeTitle(0), "Stream muted");
  assert.equal(streamVolumeTitle(80), "Stream volume 80%");
  assert.equal(liveToastText("Display 1", true), "You are live with audio: Display 1");
  assert.equal(liveToastText("Display 1", false), "You are live: Display 1");
  assert.equal(liveSourceText("Display 1", true, 30), "Streaming Display 1 with audio - 30 fps");
  assert.equal(liveSourceText("Display 1", false, 30), "Streaming Display 1 - 30 fps");
  assert.equal(liveSourceText(null, true, 12), "12 fps");
});
