import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  TTS_COMMAND, TTS_MODES, DEFAULT_TTS_MODE, TTS_RATE_MIN, TTS_RATE_MAX, DEFAULT_TTS_RATE, TTS_MAX_SPOKEN_CHARS,
  TTS_PITCH_MIN, TTS_PITCH_MAX, DEFAULT_TTS_PITCH,
  parseTtsPrefix, canSendTts, clampRate, clampPitch, readTtsPrefs, shouldSpeakIncoming, spokenText, ttsCommandHint,
  ttsVoices, ttsVoiceLabel, chosenTtsVoice,
} = require("../scripts/tts.js");
const { COMPOSER_COMMANDS, suggestTokenAt, commandSuggestions, suggestionInsert } = require("../scripts/composer-suggest.js");

test("a /tts prefix marks the message and leaves the rest as the text", () => {
  assert.deepEqual(parseTtsPrefix("/tts hello there"), { tts: true, content: "hello there" });
  assert.deepEqual(parseTtsPrefix("/TTS   shouting  "), { tts: true, content: "shouting" });
  assert.deepEqual(parseTtsPrefix("/tts\nover two\nlines"), { tts: true, content: "over two\nlines" });
  assert.deepEqual(parseTtsPrefix("/tts"), { tts: true, content: "" });
  assert.deepEqual(parseTtsPrefix("/tts   "), { tts: true, content: "" });
});

test("anything else is plain text, including look-alikes", () => {
  assert.deepEqual(parseTtsPrefix("hello /tts there"), { tts: false, content: "hello /tts there" });
  assert.deepEqual(parseTtsPrefix("/ttsx now"), { tts: false, content: "/ttsx now" });
  assert.deepEqual(parseTtsPrefix("/tt s"), { tts: false, content: "/tt s" });
  assert.deepEqual(parseTtsPrefix(" /tts late"), { tts: false, content: "/tts late" });
  assert.deepEqual(parseTtsPrefix(""), { tts: false, content: "" });
  assert.deepEqual(parseTtsPrefix(null), { tts: false, content: "" });
});

test("the permission gate reads the server view and defaults to allowed", () => {
  assert.equal(canSendTts(null), true);
  assert.equal(canSendTts({}), true);
  assert.equal(canSendTts({ can_send_tts_messages: true }), true);
  assert.equal(canSendTts({ can_send_tts_messages: false }), false);
});

test("/tts is offered as a composer command next to /gif", () => {
  assert.ok(COMPOSER_COMMANDS.some(c => c.name === TTS_COMMAND && c.needs === "tts"));
  assert.deepEqual(commandSuggestions(suggestTokenAt("/t"), { tts: true }), [{ kind: "command", name: "tts", args: "", ready: false, available: true }]);
  assert.deepEqual(commandSuggestions(suggestTokenAt("/tts hi all"), { tts: false }), [{ kind: "command", name: "tts", args: "hi all", ready: true, available: false }]);
  assert.deepEqual(commandSuggestions(suggestTokenAt("/"), { gifs: true, tts: true }).map(c => c.name), ["gif", "tts"]);
  assert.equal(suggestionInsert({ kind: "command", name: "tts" }), "/tts ");
  assert.equal(ttsCommandHint({ available: false, args: "" }), "You can't send text-to-speech messages here");
  assert.equal(ttsCommandHint({ available: true, args: "" }), "Read a message aloud for everyone");
  assert.equal(ttsCommandHint({ available: true, args: "hi" }), "Send and read aloud");
});

test("settings read back with safe defaults", () => {
  const defaults = { mode: DEFAULT_TTS_MODE, rate: DEFAULT_TTS_RATE, pitch: DEFAULT_TTS_PITCH, voice: null };
  assert.deepEqual(readTtsPrefs(null), defaults);
  assert.equal(DEFAULT_TTS_MODE, "open");
  assert.deepEqual(readTtsPrefs("not json"), defaults);
  assert.deepEqual(readTtsPrefs(JSON.stringify({ mode: "all", rate: 1.5 })), { ...defaults, mode: "all", rate: 1.5 });
  assert.deepEqual(readTtsPrefs(JSON.stringify({ mode: "never" })), { ...defaults, mode: "never" });
  assert.deepEqual(readTtsPrefs(JSON.stringify({ mode: "loud", rate: 9, pitch: -3 })), { ...defaults, rate: TTS_RATE_MAX, pitch: TTS_PITCH_MIN });
  assert.deepEqual(readTtsPrefs(JSON.stringify({ voice: "zira", pitch: 1.4 })), { ...defaults, voice: "zira", pitch: 1.4 });
  assert.deepEqual(readTtsPrefs(JSON.stringify({ voice: "" })), defaults);
  assert.deepEqual(readTtsPrefs(JSON.stringify({ voice: 7 })), defaults);
  assert.deepEqual(TTS_MODES.map(m => m.id), ["never", "open", "all"]);
  assert.ok(TTS_MODES.every(m => m.label));
});

test("the pitch stays within what a voice can do", () => {
  assert.equal(clampPitch(0), TTS_PITCH_MIN);
  assert.equal(clampPitch(4), TTS_PITCH_MAX);
  assert.equal(clampPitch("1.26"), 1.3);
  assert.equal(clampPitch(null), DEFAULT_TTS_PITCH);
  assert.equal(clampPitch(NaN), DEFAULT_TTS_PITCH);
});

test("voices list once each, your language first, then by language and name", () => {
  const raw = [
    { id: "b", name: "Microsoft Hedda", lang: "de-DE" },
    { id: "z", name: "Microsoft Zira", lang: "en-US", default: true },
    { id: "d", name: "Microsoft David", lang: "en-US" },
    { id: "z", name: "Duplicate", lang: "en-US" },
    { id: "", name: "No id", lang: "en-US" },
    { id: "g", name: "Microsoft George - English (United Kingdom)", lang: "en-GB" },
    null,
  ];
  const voices = ttsVoices(raw, "en-US");
  assert.deepEqual(voices.map(v => v.id), ["g", "d", "z", "b"]);
  assert.equal(voices.find(v => v.id === "z").default, true);
  assert.deepEqual(ttsVoices(raw, "de").map(v => v.id), ["b", "g", "d", "z"]);
  assert.deepEqual(ttsVoices(null, "en"), []);
  assert.deepEqual(ttsVoices([{ id: "x" }]), [{ id: "x", name: "x", lang: "", default: false }]);
});

test("voice labels are short and say the language", () => {
  assert.equal(ttsVoiceLabel({ name: "Microsoft Zira", lang: "en-US" }), "Zira (en-US)");
  assert.equal(ttsVoiceLabel({ name: "Microsoft George - English (United Kingdom)", lang: "en-GB" }), "George (en-GB)");
  assert.equal(ttsVoiceLabel({ name: "Samantha", lang: "" }), "Samantha");
});

test("a saved voice that is gone falls back to the computer default", () => {
  const voices = ttsVoices([{ id: "z", name: "Zira", lang: "en-US" }]);
  assert.equal(chosenTtsVoice(voices, "z").name, "Zira");
  assert.equal(chosenTtsVoice(voices, "gone"), null);
  assert.equal(chosenTtsVoice(voices, null), null);
  assert.equal(chosenTtsVoice([], "z"), null);
});

test("the rate stays within what a voice can do", () => {
  assert.equal(clampRate(0.1), TTS_RATE_MIN);
  assert.equal(clampRate(5), TTS_RATE_MAX);
  assert.equal(clampRate("1.25"), 1.3);
  assert.equal(clampRate("nope"), DEFAULT_TTS_RATE);
  assert.equal(clampRate(undefined), DEFAULT_TTS_RATE);
});

test("the setting matrix decides what gets read aloud", () => {
  const base = { mode: "all", tts: true, ownMessage: false, deafened: false, viewing: false, blocked: false, presence: "online" };
  assert.equal(shouldSpeakIncoming(base), true);
  assert.equal(shouldSpeakIncoming({ ...base, mode: "open" }), false);
  assert.equal(shouldSpeakIncoming({ ...base, mode: "open", viewing: true }), true);
  assert.equal(shouldSpeakIncoming({ ...base, mode: "never", viewing: true }), false);
  assert.equal(shouldSpeakIncoming({ ...base, tts: false }), false);
  assert.equal(shouldSpeakIncoming({ ...base, ownMessage: true }), false);
  assert.equal(shouldSpeakIncoming({ ...base, deafened: true }), false);
  assert.equal(shouldSpeakIncoming({ ...base, blocked: true }), false);
  assert.equal(shouldSpeakIncoming({ ...base, presence: "do_not_disturb" }), false);
  assert.equal(shouldSpeakIncoming({ ...base, presence: undefined }), true);
});

test("what is spoken names the author and strips markup", () => {
  const plain = text => text.replace(/\*\*/g, "");
  assert.equal(spokenText("nova", "**raid** tonight", plain), "nova said: raid tonight");
  assert.equal(spokenText("nova", "  spaced \n out  ", plain), "nova said: spaced out");
  assert.equal(spokenText("", "hi", plain), "Someone said: hi");
  assert.equal(spokenText("nova", "   ", plain), null);
  assert.equal(spokenText("nova", "", plain), null);
  assert.equal(spokenText("nova", "no plain fn"), "nova said: no plain fn");
  const long = "a".repeat(TTS_MAX_SPOKEN_CHARS + 50);
  const spoken = spokenText("nova", long, plain);
  assert.ok(spoken.endsWith(", and more"));
  assert.ok(spoken.length < long.length);
});
