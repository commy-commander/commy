import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  SOUND_STORE_MAX_BYTES, SOUND_TRIM_RATE, SOUND_TRIM_MIN_SECONDS,
  isStorableSound, initialSoundSelection, moveSoundSelection, keepsOriginalSound, trimmedSoundChannels,
  soundWaveformPeaks, fadeSoundEdges, encodeWav, formatTrimTime, soundNameDraft,
} = require("../scripts/sound-trim.js");

const bytes = text => new Uint8Array([...text].map(c => c.charCodeAt(0)));
const mp3 = new Uint8Array([0x49, 0x44, 0x33, 4, 0, 0]);

test("only formats the backend stores can be kept as they are", () => {
  assert.ok(isStorableSound(bytes("OggS\0\0")));
  assert.ok(isStorableSound(bytes("RIFF\0\0\0\0WAVEfmt ")));
  assert.ok(isStorableSound(mp3));
  assert.ok(isStorableSound(new Uint8Array([0xff, 0xfb, 0x90])));
  assert.ok(!isStorableSound(bytes("\0\0\0 ftypM4A ")));
  assert.ok(!isStorableSound(bytes("fLaC")));
  assert.ok(!isStorableSound(new Uint8Array()));
});

test("a long clip starts with the first allowed stretch selected", () => {
  assert.deepEqual(initialSoundSelection(180, 15), { start: 0, end: 15 });
  assert.deepEqual(initialSoundSelection(4, 15), { start: 0, end: 4 });
});

test("dragging a handle never selects more than the longest sound", () => {
  const sel = { start: 10, end: 20 };
  assert.deepEqual(moveSoundSelection(sel, "start", 2, 180, 15), { start: 2, end: 17 });
  assert.deepEqual(moveSoundSelection(sel, "end", 40, 180, 15), { start: 25, end: 40 });
  assert.deepEqual(moveSoundSelection(sel, "end", 500, 180, 15), { start: 165, end: 180 });
  assert.deepEqual(moveSoundSelection(sel, "start", -3, 180, 15), { start: 0, end: 15 });
});

test("the handles cannot cross or squeeze the sound to nothing", () => {
  const sel = { start: 10, end: 20 };
  assert.deepEqual(moveSoundSelection(sel, "start", 25, 180, 15), { start: 20 - SOUND_TRIM_MIN_SECONDS, end: 20 });
  assert.deepEqual(moveSoundSelection(sel, "end", 5, 180, 15), { start: 10, end: 10 + SOUND_TRIM_MIN_SECONDS });
});

test("dragging the selected part slides it without changing its length", () => {
  const sel = { start: 10, end: 20 };
  assert.deepEqual(moveSoundSelection(sel, "both", 50, 180, 15), { start: 50, end: 60 });
  assert.deepEqual(moveSoundSelection(sel, "both", 175, 180, 15), { start: 170, end: 180 });
  assert.deepEqual(moveSoundSelection(sel, "both", -4, 180, 15), { start: 0, end: 10 });
});

test("an untouched short clip uploads as it is, anything trimmed is re-encoded", () => {
  const whole = { start: 0, end: 6 };
  assert.ok(keepsOriginalSound({ bytes: mp3, duration: 6, sel: whole, maxSeconds: 15 }));
  assert.ok(!keepsOriginalSound({ bytes: mp3, duration: 6, sel: { start: 1, end: 6 }, maxSeconds: 15 }));
  assert.ok(!keepsOriginalSound({ bytes: mp3, duration: 6, sel: { start: 0, end: 5 }, maxSeconds: 15 }));
  assert.ok(!keepsOriginalSound({ bytes: mp3, duration: 40, sel: { start: 0, end: 40 }, maxSeconds: 15 }));
  assert.ok(!keepsOriginalSound({ bytes: bytes("fLaC"), duration: 6, sel: whole, maxSeconds: 15 }));
  const big = new Uint8Array(SOUND_STORE_MAX_BYTES + 1);
  big.set(mp3);
  assert.ok(!keepsOriginalSound({ bytes: big, duration: 6, sel: whole, maxSeconds: 15 }));
});

test("stereo is kept only while it fits, and the longest sound always fits in mono", () => {
  assert.equal(trimmedSoundChannels(5, 2), 2);
  assert.equal(trimmedSoundChannels(15, 2), 1);
  assert.equal(trimmedSoundChannels(5, 1), 1);
  assert.ok(44 + Math.ceil(15.2 * SOUND_TRIM_RATE) * 2 <= SOUND_STORE_MAX_BYTES);
});

test("waveform bars follow the loudest sample in each slice, scaled to the loudest bar", () => {
  const left = new Float32Array([0, 0.1, 0, 0.5, 0, 0, 0, 0]);
  const right = new Float32Array([0, 0, 0, 0, 0, 0, -0.25, 0]);
  assert.deepEqual(soundWaveformPeaks([left, right], 4), [0.2, 1, 0, 0.5]);
  assert.deepEqual(soundWaveformPeaks([new Float32Array(8)], 2), [0, 0]);
  assert.deepEqual(soundWaveformPeaks([], 3), [0, 0, 0]);
});

test("cut edges fade so a trimmed clip does not click", () => {
  const data = new Float32Array(SOUND_TRIM_RATE).fill(1);
  fadeSoundEdges([data], SOUND_TRIM_RATE);
  assert.equal(data[0], 0);
  assert.equal(data.at(-1), 0);
  assert.equal(data[SOUND_TRIM_RATE / 2], 1);
});

test("encoded clips are 16-bit PCM WAV files the backend accepts", () => {
  const left = new Float32Array([0, 1, -1, 2]);
  const right = new Float32Array([0.5, -0.5, 0, 0]);
  const wav = encodeWav([left, right], 32000);
  const view = new DataView(wav.buffer);
  assert.ok(isStorableSound(wav));
  assert.equal(wav.length, 44 + 4 * 2 * 2);
  assert.equal(view.getUint32(4, true), wav.length - 8);
  assert.equal(view.getUint16(22, true), 2);
  assert.equal(view.getUint32(24, true), 32000);
  assert.equal(view.getUint32(28, true), 128000);
  assert.equal(view.getUint32(40, true), 16);
  assert.deepEqual([0, 1, 2, 3, 4, 5].map(i => view.getInt16(44 + i * 2, true)), [0, 16384, 32767, -16384, -32768, 0]);
  assert.equal(view.getInt16(44 + 6 * 2, true), 32767);
});

test("times read as minutes, seconds and tenths", () => {
  assert.equal(formatTrimTime(0), "0:00.0");
  assert.equal(formatTrimTime(3.25), "0:03.2");
  assert.equal(formatTrimTime(75.99), "1:15.9");
  assert.equal(formatTrimTime(-1), "0:00.0");
  assert.equal(formatTrimTime(NaN), "0:00.0");
});

test("typing a name turns spaces into underscores and drops other characters", () => {
  assert.equal(soundNameDraft("Big Cheer!"), "big_cheer");
  assert.equal(soundNameDraft("air-horn 2"), "air_horn_2");
  assert.equal(soundNameDraft("x".repeat(40)).length, 32);
});
