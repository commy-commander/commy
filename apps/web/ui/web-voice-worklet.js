"use strict";

const VOICE_RATE = 48000;
const VOICE_FRAME = 480;
const SPEAKER_START = 2880;
const SPEAKER_MOST = 14400;

class MicFramer {
  constructor(rate, send) {
    this.step = rate / VOICE_RATE;
    this.pos = 0;
    this.prev = 0;
    this.frame = new Int16Array(VOICE_FRAME);
    this.filled = 0;
    this.send = send;
  }

  push(input) {
    if (!input.length) return;
    const last = input.length - 1;
    let t = this.pos;
    while (t <= last) {
      const i = Math.floor(t);
      const a = i < 0 ? this.prev : input[i];
      const b = i < last ? input[i + 1] : a;
      this.add(a + (b - a) * (t - i));
      t += this.step;
    }
    this.pos = t - input.length;
    this.prev = input[last];
  }

  add(sample) {
    this.frame[this.filled++] = Math.round(Math.max(-1, Math.min(1, sample)) * 32767);
    if (this.filled === VOICE_FRAME) {
      this.send(this.frame);
      this.frame = new Int16Array(VOICE_FRAME);
      this.filled = 0;
    }
  }
}

class SpeakerQueue {
  constructor(rate) {
    this.step = VOICE_RATE / rate;
    this.frames = [];
    this.head = null;
    this.offset = 0;
    this.queued = 0;
    this.playing = false;
    this.pos = 1;
    this.a = 0;
    this.b = 0;
  }

  add(frame) {
    this.frames.push(frame);
    this.queued += frame.length;
    while (this.queued > SPEAKER_MOST && this.frames.length > 1) {
      this.queued -= this.frames.shift().length;
    }
  }

  next() {
    while (!this.head || this.offset >= this.head.length) {
      if (!this.frames.length) return null;
      this.head = this.frames.shift();
      this.offset = 0;
    }
    this.queued -= 1;
    return this.head[this.offset++] / 32768;
  }

  fill(out) {
    if (!this.playing && this.queued < SPEAKER_START) {
      out.fill(0);
      return;
    }
    this.playing = true;
    for (let i = 0; i < out.length; i++) {
      while (this.pos >= 1) {
        const sample = this.next();
        if (sample === null) {
          this.playing = false;
          out.fill(0, i);
          return;
        }
        this.a = this.b;
        this.b = sample;
        this.pos -= 1;
      }
      out[i] = this.a + (this.b - this.a) * this.pos;
      this.pos += this.step;
    }
  }
}

if (typeof registerProcessor === "function") {
  registerProcessor("commy-mic", class extends AudioWorkletProcessor {
    constructor() {
      super();
      this.framer = new MicFramer(sampleRate, frame => this.port.postMessage(frame, [frame.buffer]));
    }

    process(inputs) {
      const channel = inputs[0] && inputs[0][0];
      if (channel) this.framer.push(channel);
      return true;
    }
  });

  registerProcessor("commy-speaker", class extends AudioWorkletProcessor {
    constructor() {
      super();
      this.queue = new SpeakerQueue(sampleRate);
      this.port.onmessage = e => this.queue.add(e.data);
    }

    process(_inputs, outputs) {
      const channel = outputs[0] && outputs[0][0];
      if (channel) this.queue.fill(channel);
      return true;
    }
  });
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { MicFramer, SpeakerQueue, VOICE_RATE, VOICE_FRAME, SPEAKER_START, SPEAKER_MOST };
}
