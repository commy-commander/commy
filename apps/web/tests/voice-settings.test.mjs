import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const voiceScripts = require("../ui/web-voice.js");
const { WEB_DEFAULTS } = require("../ui/web-commands.js");
const { WEB_STREAM_SPEAKER, WEB_VOICE_PREFS_KEY, WEB_VOICE_COMMANDS, WEB_SILENCE_DB, webVoiceDefaults, webVoicePrefs, webVoiceApply, webPeerGain, webMicConstraints, webDeviceNames, webDeviceId, webLevelDb, webVoice } = voiceScripts;

const repo = join(import.meta.dirname, "..", "..", "..");
const read = (...path) => readFileSync(join(repo, ...path), "utf8");
const alice = "6f1c2d4e-0000-4000-8000-000000000001";

test("saved voice choices are read back safely, whatever the browser storage holds", () => {
  assert.deepEqual(webVoicePrefs(null), webVoiceDefaults());
  assert.deepEqual(webVoicePrefs("{not json"), webVoiceDefaults());
  const prefs = webVoicePrefs(JSON.stringify({
    noise_cancellation: false, echo_cancellation: "yes", input_gain: 9, output_volume: -1, input_device: "Headset Mic", output_device: 4,
    user_volumes: { [alice]: 1.5, "<img>": 1 }, muted_users: [alice, alice, "x y"],
  }));
  assert.equal(prefs.noise_cancellation, false);
  assert.equal(prefs.echo_cancellation, true);
  assert.equal(prefs.input_gain, 2);
  assert.equal(prefs.output_volume, 0);
  assert.equal(prefs.input_device, "Headset Mic");
  assert.equal(prefs.output_device, null);
  assert.deepEqual(prefs.user_volumes, { [alice]: 1.5 });
  assert.deepEqual(prefs.muted_users, [alice]);
});

test("each voice setting the app changes lands in this browser's choices", () => {
  let prefs = webVoiceDefaults();
  prefs = webVoiceApply(prefs, "set_noise_cancellation", { enabled: false });
  prefs = webVoiceApply(prefs, "set_echo_cancellation", { enabled: false });
  prefs = webVoiceApply(prefs, "set_auto_level", { enabled: false });
  prefs = webVoiceApply(prefs, "set_volumes", { outputVolume: 0.5, inputGain: 1.25 });
  prefs = webVoiceApply(prefs, "set_audio_devices", { input: "Headset Mic", output: null });
  prefs = webVoiceApply(prefs, "set_user_voice", { userId: alice, volume: 0.3, muted: true });
  assert.deepEqual(prefs, { ...webVoiceDefaults(), noise_cancellation: false, echo_cancellation: false, auto_level: false, output_volume: 0.5, input_gain: 1.25, input_device: "Headset Mic", user_volumes: { [alice]: 0.3 }, muted_users: [alice] });
  assert.equal(webPeerGain(prefs, alice), 0);
  prefs = webVoiceApply(prefs, "set_user_voice", { userId: alice, volume: null, muted: false });
  assert.equal(webPeerGain(prefs, alice), 0.3);
  assert.equal(webPeerGain(prefs, "someone-else"), 1);
  assert.throws(() => webVoiceApply(prefs, "set_user_voice", { userId: "not an id" }));
});

test("stream sound follows the stream volume, apart from the person's own voice volume", () => {
  let prefs = webVoiceApply(webVoiceDefaults(), "set_user_voice", { userId: alice, volume: 0.3, muted: true });
  prefs = webVoiceApply(prefs, "set_stream_volume", { volume: 0.6 });
  assert.equal(prefs.stream_volume, 0.6);
  assert.equal(webPeerGain(prefs, `${WEB_STREAM_SPEAKER}${alice}`), 0.6);
  assert.equal(webPeerGain(prefs, alice), 0);
  assert.equal(webVoicePrefs({ stream_volume: "loud" }).stream_volume, 1);
  assert.equal(webVoiceApply(prefs, "set_stream_volume", { volume: 99 }).stream_volume, webVoiceApply(prefs, "set_stream_volume", { volume: 98 }).stream_volume);
});

test("the microphone is asked for with the chosen device and the chosen browser processing", () => {
  const prefs = { ...webVoiceDefaults(), noise_cancellation: false, auto_level: false };
  assert.deepEqual(webMicConstraints(prefs, "dev-1").audio, { echoCancellation: true, noiseSuppression: false, autoGainControl: false, channelCount: 1, deviceId: { exact: "dev-1" } });
  assert.equal(webMicConstraints(prefs).audio.deviceId, undefined);
});

test("devices are listed by name, without the browser's stand-in entries", () => {
  const devices = [
    { kind: "audioinput", deviceId: "default", label: "Default - Headset Mic" },
    { kind: "audioinput", deviceId: "a1", label: "Headset Mic" },
    { kind: "audioinput", deviceId: "a2", label: "" },
    { kind: "audioinput", deviceId: "a3", label: "Headset Mic" },
    { kind: "audiooutput", deviceId: "communications", label: "Communications" },
    { kind: "audiooutput", deviceId: "o1", label: "Speakers" },
    { kind: "videoinput", deviceId: "v1", label: "Camera" },
  ];
  assert.deepEqual(webDeviceNames(devices, "audioinput"), ["Headset Mic"]);
  assert.deepEqual(webDeviceNames(devices, "audiooutput"), ["Speakers"]);
  assert.equal(webDeviceId(devices, "audioinput", "Headset Mic"), "a1");
  assert.equal(webDeviceId(devices, "audiooutput", "Default - Headset Mic"), null);
  assert.equal(webDeviceId(devices, "audioinput", null), null);
});

test("the mic level is read in decibels, with silence at the floor", () => {
  assert.equal(webLevelDb(new Float32Array(128)), WEB_SILENCE_DB);
  assert.equal(webLevelDb(new Float32Array(0)), WEB_SILENCE_DB);
  assert.ok(Math.abs(webLevelDb(new Float32Array(128).fill(0.5)) + 6.02) < 0.01);
});

function fakeWindow(devices = []) {
  const node = () => ({ port: {}, connect(next) { return next; }, disconnect() {} });
  const win = {
    asked: [],
    tracks: [],
    timers: [],
    sinks: [],
    setTimeout(callback) { win.timers.push(callback); },
    AudioContext: class {
      constructor() { this.state = "running"; this.audioWorklet = { addModule: async () => {} }; this.destination = node(); }
      createMediaStreamSource() { return node(); }
      createGain() { return { ...node(), gain: { value: 1 } }; }
      createAnalyser() { return { ...node(), fftSize: 64, getFloatTimeDomainData(out) { out.fill(0.5); } }; }
      async setSinkId(id) { win.sinks.push(id); }
    },
    AudioWorkletNode: class { constructor() { Object.assign(this, node()); } },
    navigator: {
      mediaDevices: {
        async enumerateDevices() { return devices; },
        async getUserMedia(constraints) {
          win.asked.push(constraints.audio);
          const track = { stopped: false, stop() { this.stopped = true; } };
          win.tracks.push(track);
          return { getTracks: () => [track], getAudioTracks: () => [track] };
        },
      },
    },
  };
  return win;
}

test("turning noise or echo handling off reopens the open mic with the new choice, a volume change does not", async () => {
  const win = fakeWindow([{ kind: "audioinput", deviceId: "a1", label: "Headset Mic" }]);
  const voice = webVoice(win);
  await voice.start(() => {});
  assert.equal(win.asked.length, 1);
  await voice.configure(webVoiceApply(webVoiceDefaults(), "set_volumes", { outputVolume: 0.5, inputGain: 1.5 }));
  assert.equal(win.asked.length, 1);
  await voice.configure({ ...webVoiceDefaults(), noise_cancellation: false, input_device: "Headset Mic" });
  assert.equal(win.asked.length, 2);
  assert.equal(win.tracks[0].stopped, true);
  assert.equal(win.asked[1].noiseSuppression, false);
  assert.deepEqual(win.asked[1].deviceId, { exact: "a1" });
});

test("a mic test opens the mic outside a call and closes it after, but leaves a call's mic open", async () => {
  const win = fakeWindow();
  const voice = webVoice(win);
  assert.equal(voice.level(), WEB_SILENCE_DB);
  assert.equal(await voice.startTest(), true);
  assert.equal(win.asked.length, 1);
  assert.ok(voice.level() > -7);
  voice.stopTest();
  assert.equal(win.tracks[0].stopped, true);
  await voice.start(() => {});
  await voice.startTest();
  assert.equal(win.asked.length, 2);
  voice.stopTest();
  assert.equal(win.tracks[1].stopped, false);
  voice.stop();
  assert.equal(win.tracks[1].stopped, true);
});

test("the speakers follow the chosen output where the browser can switch it", async () => {
  const win = fakeWindow([{ kind: "audiooutput", deviceId: "o1", label: "Speakers" }]);
  const voice = webVoice(win);
  await voice.configure({ ...webVoiceDefaults(), output_device: "Speakers" });
  await voice.start(() => {});
  assert.deepEqual(win.sinks, ["o1"]);
  assert.deepEqual(await voice.devices(), { inputs: [], outputs: ["Speakers"] });
  delete win.AudioContext.prototype.setSinkId;
  assert.deepEqual((await voice.devices()).outputs, []);
});

test("stream sound opens the speakers on its own when you watch without being in a call", async () => {
  const win = fakeWindow();
  let made = 0;
  const Speaker = win.AudioWorkletNode;
  win.AudioWorkletNode = class extends Speaker { constructor() { super(); made += 1; this.port = { posted: 0, postMessage() { this.posted += 1; } }; } };
  const voice = webVoice(win);
  voice.play("stream:u1", new Float32Array(960));
  voice.play("stream:u1", new Float32Array(960));
  await new Promise(resolve => setImmediate(resolve));
  voice.play("stream:u1", new Float32Array(960));
  assert.equal(made, 1);
  assert.equal(win.asked.length, 0);
});

test("push to talk, sensitivity and voice keys are kept in this browser and checked like the desktop", () => {
  const { webHotkey, webHotkeyAction } = voiceScripts;
  let prefs = webVoiceApply(webVoiceDefaults(), "set_input_mode", { mode: "push_to_talk", talkKey: "Ctrl+Alt+T", releaseMs: 9999 });
  assert.equal(prefs.input_mode, "push_to_talk");
  assert.equal(prefs.talk_key, "Ctrl+Alt+T");
  assert.equal(prefs.talk_release_ms, 2000);
  prefs = webVoiceApply(prefs, "set_sensitivity", { auto: false, thresholdDb: -300 });
  assert.equal(prefs.auto_sensitivity, false);
  assert.equal(prefs.sensitivity_db, -100);
  prefs = webVoiceApply(prefs, "set_voice_hotkeys", { mute: "Ctrl+Shift+M", deafen: "" });
  assert.equal(prefs.mute_hotkey, "Ctrl+Shift+M");
  assert.throws(() => webVoiceApply(prefs, "set_voice_hotkeys", { mute: "Ctrl+Alt+T", deafen: "" }), /already used by another Commy hotkey/);
  assert.throws(() => webVoiceApply(prefs, "set_input_mode", { mode: "push_to_talk", talkKey: "T", releaseMs: 0 }), /not a key Commy can use/);
  assert.throws(() => webVoiceApply(prefs, "set_voice_hotkeys", { mute: "Shift", deafen: "" }), /not a hotkey Commy can use for mute/);
  assert.equal(webHotkeyAction(prefs, "ctrl+alt+t"), "talk");
  assert.equal(webHotkeyAction(prefs, "Ctrl+Shift+M"), "mute");
  assert.equal(webHotkeyAction({ ...prefs, input_mode: "voice_activity" }, "Ctrl+Alt+T"), null);
  assert.equal(webHotkey("Ctrl+Shift+Space"), "Ctrl+Shift+Space");
  assert.equal(webHotkey("Banana"), null);
  const saved = webVoicePrefs(JSON.stringify(prefs));
  assert.deepEqual(saved, prefs);
  assert.deepEqual(webVoicePrefs({ talk_key: "nope", sensitivity_db: "x", input_mode: "loud", mute_hotkey: 4 }), webVoiceDefaults());
});

test("the bridge answers the voice settings the app changes, and the backend shows them", () => {
  const app = read("apps", "desktop", "ui", "scripts", "app.js");
  for (const cmd of [...WEB_VOICE_COMMANDS, "list_audio_devices", "start_mic_test", "stop_mic_test", "mic_level"]) {
    assert.ok(app.includes(`invoke("${cmd}"`) || app.includes(`saveVoiceInput("${cmd}"`), cmd);
    assert.equal(Object.prototype.hasOwnProperty.call(WEB_DEFAULTS, cmd), false, cmd);
  }
  const bridge = read("apps", "web", "ui", "web-bridge.js");
  assert.ok(bridge.includes("WEB_VOICE_COMMANDS.includes(cmd)"));
  const calls = read("apps", "web", "src", "voice_prefs.rs");
  assert.equal(/const LOCAL_VOICE_KEY: &str = "([^"]+)"/.exec(calls)[1], WEB_VOICE_PREFS_KEY);
  const fields = /const LOCAL_VOICE_FIELDS: \[&str; \d+\] = \[([^\]]+)\]/.exec(calls)[1].match(/"[a-z_]+"/g).map(f => f.slice(1, -1));
  assert.deepEqual(fields.sort(), Object.keys(webVoiceDefaults()).sort());
});

test("a voice filter is kept in this browser and the mic test plays you back through it", async () => {
  const { WEB_VOICE_FILTERS, WEB_TEST_SPEAKER } = voiceScripts;
  assert.deepEqual(WEB_VOICE_FILTERS, ["off", "higher", "lower", "robot", "radio", "cave", "whisper"]);
  assert.equal(webVoiceApply(webVoiceDefaults(), "set_voice_filter", { filter: "cave" }).voice_filter, "cave");
  assert.throws(() => webVoiceApply(webVoiceDefaults(), "set_voice_filter", { filter: "kazoo" }), /unknown voice filter/);
  assert.equal(webVoicePrefs({ voice_filter: "kazoo" }).voice_filter, "off");
  const app = read("apps", "desktop", "ui", "scripts", "voice-filters.js");
  for (const key of WEB_VOICE_FILTERS) assert.ok(app.includes(`key: "${key}"`), key);

  const win = fakeWindow();
  const nodes = [];
  win.AudioWorkletNode = class {
    constructor(ctx, name) { Object.assign(this, { name, sent: [], port: { postMessage: pcm => this.sent.push(pcm) }, connect: next => next, disconnect() {} }); nodes.push(this); }
  };
  const heard = [];
  const voice = webVoice(win, { testFrame: frame => { heard.push(frame); return Int16Array.from(frame, s => s >> 1); } });
  await voice.configure(webVoiceApply(webVoiceDefaults(), "set_voice_filter", { filter: "robot" }));
  await voice.startTest();
  const mic = nodes.find(n => n.port.onmessage);
  mic.port.onmessage({ data: new Int16Array([800, -800]) });
  assert.equal(heard.length, 1);
  const speaker = nodes.find(n => n.name === "commy-speaker");
  assert.deepEqual(Array.from(speaker.sent[0]), [400, -400]);
  assert.equal(WEB_TEST_SPEAKER, "mic-test");
  await voice.configure(webVoiceDefaults());
  mic.port.onmessage({ data: new Int16Array([800, -800]) });
  assert.equal(heard.length, 1, "with the filter off the test plays the plain mic instead");
  voice.stopTest();
});
