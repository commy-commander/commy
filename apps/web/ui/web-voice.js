"use strict";

const WEB_VOICE_WORKLET = "scripts/web-voice-worklet.js";
const MIC_RETRY_MS = 1000;
const WEB_VOICE_PREFS_KEY = "commy.web.voice";
const WEB_MAX_GAIN = 2;
const WEB_MAX_DEVICE_NAME = 200;
const WEB_MAX_TUNED_PEOPLE = 500;
const WEB_SILENCE_DB = -100;
const WEB_PSEUDO_DEVICES = ["default", "communications"];
const WEB_VOICE_COMMANDS = ["set_noise_cancellation", "set_echo_cancellation", "set_auto_level", "set_volumes", "set_audio_devices", "set_user_voice", "set_stream_volume", "set_input_mode", "set_sensitivity", "set_voice_hotkeys", "set_voice_filter"];
const WEB_VOICE_FILTERS = ["off", "higher", "lower", "robot", "radio", "cave", "whisper"];
const WEB_TEST_SPEAKER = "mic-test";
const WEB_TALK_KEY = "Ctrl+Shift+Space";
const WEB_MAX_RELEASE_MS = 2000;
const WEB_HOTKEY = /^(?:(?:Ctrl|Alt|Shift|Super)\+)+[A-Za-z0-9]{1,24}$/;
const WEB_STREAM_SPEAKER = "stream:";
const WEB_MIC_REFUSED = "Commy could not open your microphone. Allow it for this site in your browser and try again.";

function webVoiceDefaults() {
  return {
    noise_cancellation: true, echo_cancellation: true, auto_level: true, input_gain: 1, output_volume: 1, stream_volume: 1, input_device: null, output_device: null, user_volumes: {}, muted_users: [],
    input_mode: "voice_activity", talk_key: WEB_TALK_KEY, talk_release_ms: 200, auto_sensitivity: true, sensitivity_db: -45, mute_hotkey: "", deafen_hotkey: "", voice_filter: "off",
  };
}

function webHotkey(key) {
  const trimmed = typeof key === "string" ? key.trim() : "";
  return WEB_HOTKEY.test(trimmed) && !/^(?:(?:Ctrl|Alt|Shift|Super)\+)*(?:Control|Shift|Alt|Meta)$/.test(trimmed) ? trimmed : null;
}

function webHotkeyClash(prefs) {
  const keys = [prefs.input_mode === "push_to_talk" ? prefs.talk_key : "", prefs.mute_hotkey, prefs.deafen_hotkey].filter(Boolean);
  const seen = new Set();
  for (const key of keys) {
    if (seen.has(key.toLowerCase())) return key;
    seen.add(key.toLowerCase());
  }
  return null;
}

function webHotkeyAction(prefs, combo) {
  if (!combo) return null;
  const same = key => Boolean(key) && key.toLowerCase() === combo.toLowerCase();
  if (prefs.input_mode === "push_to_talk" && same(prefs.talk_key)) return "talk";
  if (same(prefs.mute_hotkey)) return "mute";
  if (same(prefs.deafen_hotkey)) return "deafen";
  return null;
}

function webGain(value, fallback) {
  const n = Number(value);
  return Number.isFinite(n) ? Math.min(WEB_MAX_GAIN, Math.max(0, n)) : fallback;
}

function webDeviceName(name) {
  return typeof name === "string" && name.trim() ? name.slice(0, WEB_MAX_DEVICE_NAME) : null;
}

function webPersonId(id) {
  return typeof id === "string" && /^[0-9a-fA-F-]{1,64}$/.test(id);
}

function webVoicePrefs(raw) {
  const prefs = webVoiceDefaults();
  let saved = null;
  try {
    saved = typeof raw === "string" ? JSON.parse(raw) : raw;
  } catch {
    saved = null;
  }
  if (!saved || typeof saved !== "object") return prefs;
  for (const key of ["noise_cancellation", "echo_cancellation", "auto_level"]) {
    if (typeof saved[key] === "boolean") prefs[key] = saved[key];
  }
  prefs.input_gain = webGain(saved.input_gain, 1);
  prefs.output_volume = webGain(saved.output_volume, 1);
  prefs.stream_volume = webGain(saved.stream_volume, 1);
  prefs.input_device = webDeviceName(saved.input_device);
  prefs.output_device = webDeviceName(saved.output_device);
  const volumes = saved.user_volumes && typeof saved.user_volumes === "object" ? Object.entries(saved.user_volumes) : [];
  for (const [id, volume] of volumes.filter(([id]) => webPersonId(id)).slice(0, WEB_MAX_TUNED_PEOPLE)) prefs.user_volumes[id] = webGain(volume, 1);
  prefs.muted_users = Array.isArray(saved.muted_users) ? [...new Set(saved.muted_users.filter(webPersonId))].slice(0, WEB_MAX_TUNED_PEOPLE) : [];
  if (saved.input_mode === "push_to_talk") prefs.input_mode = "push_to_talk";
  prefs.talk_key = webHotkey(saved.talk_key) || WEB_TALK_KEY;
  const release = Number(saved.talk_release_ms);
  if (Number.isInteger(release) && release >= 0) prefs.talk_release_ms = Math.min(WEB_MAX_RELEASE_MS, release);
  if (saved.auto_sensitivity === false) prefs.auto_sensitivity = false;
  const db = Number(saved.sensitivity_db);
  if (saved.sensitivity_db !== null && saved.sensitivity_db !== "" && Number.isFinite(db)) prefs.sensitivity_db = Math.min(0, Math.max(-100, db));
  prefs.mute_hotkey = webHotkey(saved.mute_hotkey) || "";
  prefs.deafen_hotkey = webHotkey(saved.deafen_hotkey) || "";
  if (WEB_VOICE_FILTERS.includes(saved.voice_filter)) prefs.voice_filter = saved.voice_filter;
  return prefs;
}

function webVoiceApply(prefs, cmd, args) {
  const next = webVoicePrefs(prefs);
  const given = args || {};
  if (cmd === "set_noise_cancellation") next.noise_cancellation = Boolean(given.enabled);
  if (cmd === "set_echo_cancellation") next.echo_cancellation = Boolean(given.enabled);
  if (cmd === "set_auto_level") next.auto_level = Boolean(given.enabled);
  if (cmd === "set_volumes") {
    next.output_volume = webGain(given.outputVolume, next.output_volume);
    next.input_gain = webGain(given.inputGain, next.input_gain);
  }
  if (cmd === "set_stream_volume") next.stream_volume = webGain(given.volume, next.stream_volume);
  if (cmd === "set_audio_devices") {
    next.input_device = webDeviceName(given.input);
    next.output_device = webDeviceName(given.output);
  }
  if (cmd === "set_input_mode") {
    if (given.mode !== "voice_activity" && given.mode !== "push_to_talk") throw "unknown input mode";
    const key = webHotkey(given.talkKey);
    if (!key) throw `${String(given.talkKey || "").trim()} is not a key Commy can use for push to talk`;
    next.input_mode = given.mode;
    next.talk_key = key;
    const release = Number(given.releaseMs);
    next.talk_release_ms = Number.isFinite(release) ? Math.min(WEB_MAX_RELEASE_MS, Math.max(0, Math.round(release))) : next.talk_release_ms;
  }
  if (cmd === "set_sensitivity") {
    next.auto_sensitivity = given.auto !== false;
    const db = Number(given.thresholdDb);
    if (Number.isFinite(db)) next.sensitivity_db = Math.min(0, Math.max(-100, db));
  }
  if (cmd === "set_voice_hotkeys") {
    for (const [field, value, what] of [["mute_hotkey", given.mute, "mute"], ["deafen_hotkey", given.deafen, "deafen"]]) {
      const trimmed = typeof value === "string" ? value.trim() : "";
      if (trimmed && !webHotkey(trimmed)) throw `${trimmed} is not a hotkey Commy can use for ${what}`;
      next[field] = trimmed;
    }
  }
  if (cmd === "set_voice_filter") {
    if (!WEB_VOICE_FILTERS.includes(given.filter)) throw "unknown voice filter";
    next.voice_filter = given.filter;
  }
  const clash = webHotkeyClash(next);
  if (clash) throw `${clash} is already used by another Commy hotkey`;
  if (cmd === "set_user_voice") {
    if (!webPersonId(given.userId)) throw "unknown member";
    const id = given.userId;
    if (given.volume !== null && given.volume !== undefined) next.user_volumes[id] = webGain(given.volume, 1);
    if (given.muted === true && !next.muted_users.includes(id)) next.muted_users.push(id);
    if (given.muted === false) next.muted_users = next.muted_users.filter(user => user !== id);
  }
  return webVoicePrefs(next);
}

function webPeerGain(prefs, peer) {
  if (peer.startsWith(WEB_STREAM_SPEAKER)) return prefs.stream_volume;
  return prefs.muted_users.includes(peer) ? 0 : prefs.user_volumes[peer] ?? 1;
}

function webMicConstraints(prefs = webVoiceDefaults(), deviceId = null) {
  const audio = { echoCancellation: prefs.echo_cancellation, noiseSuppression: prefs.noise_cancellation, autoGainControl: prefs.auto_level, channelCount: 1 };
  if (deviceId) audio.deviceId = { exact: deviceId };
  return { audio };
}

function webMicShape(prefs) {
  return [prefs.echo_cancellation, prefs.noise_cancellation, prefs.auto_level, prefs.input_device].join("|");
}

function webDeviceNames(devices, kind) {
  const names = devices
    .filter(d => d.kind === kind && !WEB_PSEUDO_DEVICES.includes(d.deviceId))
    .map(d => webDeviceName(d.label))
    .filter(Boolean);
  return [...new Set(names)];
}

function webDeviceId(devices, kind, name) {
  if (!name) return null;
  const found = devices.find(d => d.kind === kind && !WEB_PSEUDO_DEVICES.includes(d.deviceId) && webDeviceName(d.label) === name);
  return found ? found.deviceId : null;
}

function webLevelDb(samples) {
  if (!samples.length) return WEB_SILENCE_DB;
  let sum = 0;
  for (const s of samples) sum += s * s;
  const rms = Math.sqrt(sum / samples.length);
  return rms > 0 ? Math.max(WEB_SILENCE_DB, 20 * Math.log10(rms)) : WEB_SILENCE_DB;
}

function webVoice(win, hooks = {}) {
  let context = null;
  let master = null;
  let parts = null;
  let opening = null;
  let generation = 0;
  let sendFrame = null;
  let testing = false;
  let prefs = webVoiceDefaults();
  const speakers = new Map();
  const speech = new Set();
  let speechEnds = 0;

  const mediaDevices = () => win.navigator.mediaDevices;
  const canPickOutput = () => typeof win.AudioContext === "function" && typeof win.AudioContext.prototype.setSinkId === "function";

  async function listDevices() {
    try {
      return (await mediaDevices().enumerateDevices()) || [];
    } catch {
      return [];
    }
  }

  async function applyOutput() {
    if (!context || typeof context.setSinkId !== "function") return;
    const id = webDeviceId(await listDevices(), "audiooutput", prefs.output_device);
    await context.setSinkId(id || "").catch(() => {});
  }

  async function ready() {
    opening ||= (async () => {
      context = new win.AudioContext({ latencyHint: "interactive" });
      await context.audioWorklet.addModule(WEB_VOICE_WORKLET);
      master = context.createGain();
      master.gain.value = prefs.output_volume;
      master.connect(context.destination);
      await applyOutput();
    })().catch(e => {
      opening = null;
      master = null;
      throw e;
    });
    await opening;
    if (context.state === "suspended") await context.resume().catch(() => {});
    return context;
  }

  function hearsFilter() {
    return prefs.voice_filter !== "off" && typeof hooks.testFrame === "function";
  }

  function monitorOn() {
    if (!parts || parts.monitor || hearsFilter()) return;
    parts.monitor = context.createGain();
    parts.gain.connect(parts.monitor).connect(master);
  }

  function monitorOff() {
    if (!parts || !parts.monitor) return;
    parts.gain.disconnect(parts.monitor);
    parts.monitor.disconnect();
    parts.monitor = null;
  }

  function release() {
    if (!parts) return;
    monitorOff();
    parts.mic.port.onmessage = null;
    parts.source.disconnect();
    parts.gain.disconnect();
    parts.mic.disconnect();
    parts.hush.disconnect();
    for (const track of parts.stream.getTracks()) {
      track.onended = null;
      track.stop();
    }
    parts = null;
  }

  async function askForMic() {
    const id = webDeviceId(await listDevices(), "audioinput", prefs.input_device);
    try {
      return await mediaDevices().getUserMedia(webMicConstraints(prefs, id));
    } catch (e) {
      if (!id) throw e;
      return mediaDevices().getUserMedia(webMicConstraints(prefs));
    }
  }

  async function openMic(mine) {
    const ctx = await ready();
    let stream;
    try {
      stream = await askForMic();
    } catch {
      return false;
    }
    if (mine !== generation) {
      for (const track of stream.getTracks()) track.stop();
      return false;
    }
    const source = ctx.createMediaStreamSource(stream);
    const gain = ctx.createGain();
    gain.gain.value = prefs.input_gain;
    const analyser = ctx.createAnalyser ? ctx.createAnalyser() : null;
    const mic = new win.AudioWorkletNode(ctx, "commy-mic", { numberOfInputs: 1, numberOfOutputs: 1, channelCount: 1, channelCountMode: "explicit" });
    const hush = ctx.createGain();
    hush.gain.value = 0;
    mic.port.onmessage = e => {
      if (sendFrame) sendFrame(e.data);
      if (testing && hearsFilter()) {
        const heard = hooks.testFrame(e.data);
        if (heard && heard.length) play(WEB_TEST_SPEAKER, heard);
      }
    };
    source.connect(gain);
    if (analyser) gain.connect(analyser);
    gain.connect(mic).connect(hush).connect(ctx.destination);
    parts = { stream, source, gain, analyser, mic, hush, monitor: null, shape: webMicShape(prefs) };
    if (testing) monitorOn();
    for (const track of stream.getAudioTracks()) track.onended = () => reopenMic(mine);
    return true;
  }

  function reopenMic(mine) {
    if (mine !== generation) return;
    release();
    win.setTimeout(async () => {
      if (mine === generation && !(await openMic(mine))) reopenMic(mine);
    }, MIC_RETRY_MS);
  }

  function restartMic() {
    generation += 1;
    release();
    return openMic(generation);
  }

  function stopSpeakers() {
    for (const speaker of speakers.values()) {
      speaker.node.disconnect();
      speaker.gain.disconnect();
    }
    speakers.clear();
  }

  function stop() {
    sendFrame = null;
    stopSpeakers();
    if (testing) return;
    generation += 1;
    release();
  }

  async function start(onFrame) {
    stop();
    sendFrame = onFrame;
    if (parts) return true;
    return restartMic();
  }

  async function startTest() {
    testing = true;
    if (parts) {
      monitorOn();
      return true;
    }
    const opened = await restartMic();
    if (!opened) testing = false;
    return opened;
  }

  function stopTest() {
    testing = false;
    monitorOff();
    if (sendFrame) return;
    generation += 1;
    release();
  }

  function level() {
    if (!parts || !parts.analyser) return WEB_SILENCE_DB;
    const samples = new Float32Array(parts.analyser.fftSize);
    parts.analyser.getFloatTimeDomainData(samples);
    return webLevelDb(samples);
  }

  async function configure(next) {
    const before = prefs;
    prefs = webVoicePrefs(next);
    if (master) master.gain.value = prefs.output_volume;
    if (parts) parts.gain.gain.value = prefs.input_gain;
    if (testing && parts) {
      if (hearsFilter()) monitorOff();
      else monitorOn();
    }
    for (const [peer, speaker] of speakers) speaker.gain.gain.value = webPeerGain(prefs, peer);
    if (before.output_device !== prefs.output_device) await applyOutput();
    if (parts && parts.shape !== webMicShape(prefs)) await restartMic();
  }

  async function devices() {
    const found = await listDevices();
    return { inputs: webDeviceNames(found, "audioinput"), outputs: canPickOutput() ? webDeviceNames(found, "audiooutput") : [] };
  }

  function play(peer, pcm) {
    if (!master) {
      ready().catch(() => {});
      return;
    }
    let speaker = speakers.get(peer);
    if (!speaker) {
      const node = new win.AudioWorkletNode(context, "commy-speaker", { numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [1] });
      const gain = context.createGain();
      gain.gain.value = webPeerGain(prefs, peer);
      node.connect(gain).connect(master);
      speaker = { node, gain };
      speakers.set(peer, speaker);
    }
    speaker.node.port.postMessage(pcm, [pcm.buffer]);
  }

  async function speak(samples, rate, append) {
    const ctx = await ready();
    if (!append) hush();
    const buffer = ctx.createBuffer(1, samples.length, rate);
    buffer.copyToChannel(samples, 0);
    const source = ctx.createBufferSource();
    source.buffer = buffer;
    source.connect(master);
    const at = Math.max(ctx.currentTime, speechEnds);
    source.start(at);
    speechEnds = at + buffer.duration;
    speech.add(source);
    source.onended = () => speech.delete(source);
  }

  function hush() {
    for (const source of speech) {
      source.onended = null;
      try {
        source.stop();
      } catch {}
      source.disconnect();
    }
    speech.clear();
    speechEnds = 0;
  }

  return { start, stop, play, speak, hush, startTest, stopTest, level, configure, devices };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { WEB_VOICE_FILTERS, WEB_TEST_SPEAKER, WEB_TALK_KEY, webHotkey, webHotkeyAction, WEB_STREAM_SPEAKER, WEB_VOICE_WORKLET, WEB_VOICE_PREFS_KEY, WEB_VOICE_COMMANDS, WEB_MIC_REFUSED, WEB_SILENCE_DB, webVoiceDefaults, webVoicePrefs, webVoiceApply, webPeerGain, webMicConstraints, webDeviceNames, webDeviceId, webLevelDb, webVoice };
}
