import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  VOICE_MESSAGE_FILENAME, VOICE_MESSAGE_MIME, MAX_VOICE_MESSAGE_SECS, WAVEFORM_BARS,
  isVoiceMessage, onlyAVoiceMessage, formatClock, waveformBars, playedBars, oggDuration, recordingTimer, voiceMessageText,
} = require("../scripts/voice-message.js");

const voice = { filename: VOICE_MESSAGE_FILENAME, mime: VOICE_MESSAGE_MIME, size_bytes: 4096, blob: "v1" };

function oggPage({ granule, body, flags = 0, sequence = 0 }) {
  const lacing = [];
  let rest = body.length;
  while (rest >= 255) { lacing.push(255); rest -= 255; }
  lacing.push(rest);
  const page = new Uint8Array(27 + lacing.length + body.length);
  page.set([0x4f, 0x67, 0x67, 0x53, 0, flags]);
  const view = new DataView(page.buffer);
  view.setUint32(6, granule % 4294967296, true);
  view.setUint32(10, Math.floor(granule / 4294967296), true);
  view.setUint32(14, 7, true);
  view.setUint32(18, sequence, true);
  page[26] = lacing.length;
  page.set(lacing, 27);
  page.set(body, 27 + lacing.length);
  return page;
}

function opusHead(preSkip) {
  const head = new Uint8Array(19);
  head.set([...("OpusHead".split("").map(c => c.charCodeAt(0)))]);
  head[8] = 1;
  head[9] = 1;
  head[10] = preSkip & 0xff;
  head[11] = preSkip >> 8;
  new DataView(head.buffer).setUint32(12, 48000, true);
  return head;
}

function concat(pages) {
  const out = new Uint8Array(pages.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of pages) { out.set(p, at); at += p.length; }
  return out;
}

test("a voice message is the fixed filename with the ogg mime, nothing else", () => {
  assert.equal(isVoiceMessage(voice), true);
  assert.equal(isVoiceMessage({ ...voice, filename: "clip.ogg" }), false);
  assert.equal(isVoiceMessage({ ...voice, mime: "audio/mpeg" }), false);
  assert.equal(isVoiceMessage(null), false);
  assert.equal(onlyAVoiceMessage("", [voice]), true);
  assert.equal(onlyAVoiceMessage("   ", [voice]), true);
  assert.equal(onlyAVoiceMessage("listen", [voice]), false);
  assert.equal(onlyAVoiceMessage("", []), false);
  assert.equal(onlyAVoiceMessage("", [voice, { ...voice, filename: "notes.txt", mime: "text/plain" }]), false);
});

test("the clock shows minutes and zero-padded seconds, never negatives or NaN", () => {
  assert.equal(formatClock(0), "0:00");
  assert.equal(formatClock(7.9), "0:07");
  assert.equal(formatClock(65), "1:05");
  assert.equal(formatClock(300), "5:00");
  assert.equal(formatClock(-3), "0:00");
  assert.equal(formatClock(NaN), "0:00");
  assert.equal(formatClock(undefined), "0:00");
});

test("waveform bars are deterministic per clip, sized within range and differ between clips", () => {
  const a = waveformBars("blob-a");
  const b = waveformBars("blob-a");
  const c = waveformBars("blob-b");
  assert.equal(a.length, WAVEFORM_BARS);
  assert.deepEqual(a, b);
  assert.notDeepEqual(a, c);
  assert.ok(a.every(h => h >= 0.1 && h <= 1));
  assert.equal(waveformBars("", 5).length, 5);
});

test("played bars follow progress and clamp at the ends", () => {
  assert.equal(playedBars(0, 28), 0);
  assert.equal(playedBars(-1, 28), 0);
  assert.equal(playedBars(0.5, 28), 14);
  assert.equal(playedBars(1, 28), 28);
  assert.equal(playedBars(4, 28), 28);
  assert.equal(playedBars(NaN, 28), 0);
});

test("ogg duration comes from the last granule position minus the pre-skip", () => {
  const preSkip = 312;
  const bytes = concat([
    oggPage({ granule: 0, body: opusHead(preSkip), flags: 2, sequence: 0 }),
    oggPage({ granule: 0, body: new Uint8Array([79, 112, 117, 115, 84, 97, 103, 115, 0, 0, 0, 0, 0, 0, 0, 0]), sequence: 1 }),
    oggPage({ granule: 48000 + preSkip, body: new Uint8Array(300).fill(1), sequence: 2 }),
    oggPage({ granule: 42 * 48000 + preSkip, body: new Uint8Array(40).fill(2), flags: 4, sequence: 3 }),
  ]);
  assert.equal(oggDuration(bytes), 42);
  assert.equal(oggDuration(bytes.buffer), 42);
  assert.equal(oggDuration(new Uint8Array([1, 2, 3])), null);
  assert.equal(oggDuration(new Uint8Array(0)), null);
  const headerless = concat([oggPage({ granule: 96000, body: new Uint8Array(10), flags: 4 })]);
  assert.equal(oggDuration(headerless), 2);
  const truncated = bytes.subarray(0, bytes.length - 5);
  assert.equal(oggDuration(truncated), 1, "a cut-off last page is ignored, the page before it counts");
});

test("the recording timer counts up and stops at five minutes", () => {
  assert.deepEqual(recordingTimer(0), { label: "0:00", remaining: MAX_VOICE_MESSAGE_SECS, done: false });
  assert.deepEqual(recordingTimer(61.4), { label: "1:01", remaining: MAX_VOICE_MESSAGE_SECS - 61.4, done: false });
  assert.deepEqual(recordingTimer(300), { label: "5:00", remaining: 0, done: true });
  assert.deepEqual(recordingTimer(301), { label: "5:00", remaining: 0, done: true });
  assert.equal(recordingTimer(NaN).label, "0:00");
});

test("the fallback text names the clip and its length when known", () => {
  assert.equal(voiceMessageText(12), "Voice message, 0:12");
  assert.equal(voiceMessageText(null), "Voice message");
  assert.equal(voiceMessageText(undefined), "Voice message");
});
