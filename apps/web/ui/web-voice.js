"use strict";

const WEB_VOICE_WORKLET = "scripts/web-voice-worklet.js";

function webMicConstraints() {
  return { audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true, channelCount: 1 } };
}

function webVoice(win) {
  let context = null;
  let parts = null;
  let generation = 0;
  const speakers = new Map();

  async function ready() {
    if (!context) {
      context = new win.AudioContext({ latencyHint: "interactive" });
      await context.audioWorklet.addModule(WEB_VOICE_WORKLET);
    }
    if (context.state === "suspended") await context.resume().catch(() => {});
    return context;
  }

  function release() {
    if (!parts) return;
    parts.mic.port.onmessage = null;
    parts.source.disconnect();
    parts.mic.disconnect();
    parts.hush.disconnect();
    for (const track of parts.stream.getTracks()) track.stop();
    parts = null;
  }

  function stop() {
    generation += 1;
    release();
    for (const node of speakers.values()) node.disconnect();
    speakers.clear();
  }

  async function start(onFrame) {
    stop();
    const mine = generation;
    const ctx = await ready();
    let stream;
    try {
      stream = await win.navigator.mediaDevices.getUserMedia(webMicConstraints());
    } catch {
      return false;
    }
    if (mine !== generation) {
      for (const track of stream.getTracks()) track.stop();
      return false;
    }
    const source = ctx.createMediaStreamSource(stream);
    const mic = new win.AudioWorkletNode(ctx, "commy-mic", { numberOfInputs: 1, numberOfOutputs: 1, channelCount: 1, channelCountMode: "explicit" });
    const hush = ctx.createGain();
    hush.gain.value = 0;
    mic.port.onmessage = e => onFrame(e.data);
    source.connect(mic).connect(hush).connect(ctx.destination);
    parts = { stream, source, mic, hush };
    return true;
  }

  function play(peer, pcm) {
    if (!context) return;
    let node = speakers.get(peer);
    if (!node) {
      node = new win.AudioWorkletNode(context, "commy-speaker", { numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [1] });
      node.connect(context.destination);
      speakers.set(peer, node);
    }
    node.port.postMessage(pcm, [pcm.buffer]);
  }

  return { start, stop, play };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { WEB_VOICE_WORKLET, webMicConstraints, webVoice };
}
