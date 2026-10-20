const VOICE_MESSAGE_FILENAME = "voice-message.ogg";
const VOICE_MESSAGE_MIME = "audio/ogg";
const VOICE_PLAYBACK_TYPE = 'audio/ogg; codecs="opus"';
const MAX_VOICE_MESSAGE_SECS = 300;
const WAVEFORM_BARS = 28;
const OPUS_SAMPLE_RATE = 48000;
const OGG_HEADER_BYTES = 27;

function isVoiceMessage(att) {
  return Boolean(att) && att.mime === VOICE_MESSAGE_MIME && att.filename === VOICE_MESSAGE_FILENAME;
}

function onlyAVoiceMessage(content, attachments) {
  const list = attachments || [];
  return list.length > 0 && list.every(isVoiceMessage) && !String(content || "").trim();
}

function formatClock(secs) {
  const whole = Number.isFinite(secs) && secs > 0 ? Math.floor(secs) : 0;
  const minutes = Math.floor(whole / 60);
  const seconds = whole % 60;
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

function hashSeed(text) {
  let h = 2166136261;
  for (const ch of String(text || "")) {
    h ^= ch.codePointAt(0);
    h = Math.imul(h, 16777619) >>> 0;
  }
  return h || 1;
}

function waveformBars(seed, count = WAVEFORM_BARS) {
  let x = hashSeed(seed);
  const bars = [];
  for (let i = 0; i < count; i++) {
    x ^= x << 13; x >>>= 0;
    x ^= x >>> 17;
    x ^= x << 5; x >>>= 0;
    const swell = 0.55 + 0.45 * Math.sin((i / count) * Math.PI);
    bars.push(Math.round((0.2 + 0.8 * (x % 1000) / 1000 * swell) * 100) / 100);
  }
  return bars;
}

function playedBars(progress, count = WAVEFORM_BARS) {
  if (!Number.isFinite(progress) || progress <= 0) return 0;
  if (progress >= 1) return count;
  return Math.round(progress * count);
}

function readU64(view, at) {
  return view.getUint32(at + 4, true) * 4294967296 + view.getUint32(at, true);
}

function oggDuration(bytes) {
  const data = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes || []);
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
  let at = 0;
  let preSkip = 0;
  let granule = null;
  let first = true;
  while (at + OGG_HEADER_BYTES <= data.length) {
    if (data[at] !== 0x4f || data[at + 1] !== 0x67 || data[at + 2] !== 0x67 || data[at + 3] !== 0x53) return null;
    const segments = data[at + 26];
    const lacing = data.subarray(at + OGG_HEADER_BYTES, at + OGG_HEADER_BYTES + segments);
    if (lacing.length < segments) return null;
    let bodyLength = 0;
    for (const l of lacing) bodyLength += l;
    const bodyAt = at + OGG_HEADER_BYTES + segments;
    if (bodyAt + bodyLength > data.length) break;
    if (first) {
      const head = data.subarray(bodyAt, bodyAt + 12);
      if (String.fromCharCode(...head.subarray(0, 8)) === "OpusHead") preSkip = head[10] | (head[11] << 8);
      first = false;
    }
    const position = readU64(view, at + 6);
    if (segments > 0 && position < 2 ** 53) granule = position;
    at = bodyAt + bodyLength;
  }
  if (granule === null) return null;
  return Math.max(0, granule - preSkip) / OPUS_SAMPLE_RATE;
}

function recordingTimer(elapsedSecs) {
  const elapsed = Number.isFinite(elapsedSecs) && elapsedSecs > 0 ? elapsedSecs : 0;
  const capped = Math.min(elapsed, MAX_VOICE_MESSAGE_SECS);
  return { label: formatClock(capped), remaining: MAX_VOICE_MESSAGE_SECS - capped, done: elapsed >= MAX_VOICE_MESSAGE_SECS };
}

function voiceMessageText(secs) {
  return Number.isFinite(secs) ? `Voice message, ${formatClock(secs)}` : "Voice message";
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    VOICE_MESSAGE_FILENAME, VOICE_MESSAGE_MIME, VOICE_PLAYBACK_TYPE, MAX_VOICE_MESSAGE_SECS, WAVEFORM_BARS,
    isVoiceMessage, onlyAVoiceMessage, formatClock, waveformBars, playedBars, oggDuration, recordingTimer, voiceMessageText,
  };
}
