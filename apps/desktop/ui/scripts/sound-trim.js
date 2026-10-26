const SOUND_STORE_MAX_BYTES = 1024 * 1024;
const SOUND_SOURCE_MAX_BYTES = 8 * 1024 * 1024;
const SOUND_SOURCE_MAX_SECONDS = 600;
const SOUND_TRIM_RATE = 32000;
const SOUND_TRIM_MIN_SECONDS = 0.3;
const SOUND_TRIM_FADE_SECONDS = 0.004;
const SOUND_WHOLE_SLACK_SECONDS = 0.05;
const SOUND_LENGTH_SLACK_SECONDS = 0.2;
const WAV_HEADER_BYTES = 44;

function isStorableSound(bytes) {
  const b = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes || []);
  const ascii = (at, text) => [...text].every((ch, i) => b[at + i] === ch.charCodeAt(0));
  if (ascii(0, "OggS")) return true;
  if (b.length >= 12 && ascii(0, "RIFF") && ascii(8, "WAVE")) return true;
  return ascii(0, "ID3") || (b.length >= 2 && b[0] === 0xff && (b[1] & 0xe0) === 0xe0);
}

function initialSoundSelection(duration, maxSeconds) {
  return { start: 0, end: Math.min(duration, maxSeconds) };
}

function moveSoundSelection(sel, which, at, duration, maxSeconds) {
  const minLen = Math.min(SOUND_TRIM_MIN_SECONDS, duration);
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
  if (which === "both") {
    const len = sel.end - sel.start;
    const start = clamp(at, 0, duration - len);
    return { start, end: start + len };
  }
  if (which === "start") {
    const start = clamp(at, 0, sel.end - minLen);
    return { start, end: Math.min(sel.end, start + maxSeconds) };
  }
  const end = clamp(at, sel.start + minLen, duration);
  return { start: Math.max(sel.start, end - maxSeconds), end };
}

function keepsOriginalSound({ bytes, duration, sel, maxSeconds }) {
  const size = bytes ? bytes.length : 0;
  return isStorableSound(bytes)
    && size <= SOUND_STORE_MAX_BYTES
    && duration <= maxSeconds + SOUND_LENGTH_SLACK_SECONDS
    && sel.start <= SOUND_WHOLE_SLACK_SECONDS
    && sel.end >= duration - SOUND_WHOLE_SLACK_SECONDS;
}

function trimmedSoundChannels(seconds, sourceChannels, rate = SOUND_TRIM_RATE) {
  const stereoBytes = WAV_HEADER_BYTES + Math.ceil(seconds * rate) * 2 * 2;
  return sourceChannels >= 2 && stereoBytes <= SOUND_STORE_MAX_BYTES ? 2 : 1;
}

function soundWaveformPeaks(channels, bars) {
  const frames = channels.length ? channels[0].length : 0;
  const peaks = new Array(bars).fill(0);
  if (!frames) return peaks;
  for (let bar = 0; bar < bars; bar++) {
    const from = Math.floor((bar * frames) / bars);
    const to = Math.max(from + 1, Math.floor(((bar + 1) * frames) / bars));
    let peak = 0;
    for (const data of channels) {
      for (let i = from; i < to && i < frames; i++) {
        const v = Math.abs(data[i]);
        if (v > peak) peak = v;
      }
    }
    peaks[bar] = peak;
  }
  const loudest = Math.max(...peaks);
  return loudest > 0 ? peaks.map(p => Math.round((p / loudest) * 100) / 100) : peaks;
}

function fadeSoundEdges(channels, rate) {
  const frames = channels.length ? channels[0].length : 0;
  const fade = Math.min(Math.floor(frames / 2), Math.round(SOUND_TRIM_FADE_SECONDS * rate));
  for (const data of channels) {
    for (let i = 0; i < fade; i++) {
      const gain = i / fade;
      data[i] *= gain;
      data[frames - 1 - i] *= gain;
    }
  }
  return channels;
}

function encodeWav(channels, rate) {
  const count = channels.length;
  const frames = count ? channels[0].length : 0;
  const dataBytes = frames * count * 2;
  const out = new Uint8Array(WAV_HEADER_BYTES + dataBytes);
  const view = new DataView(out.buffer);
  const text = (at, s) => [...s].forEach((ch, i) => view.setUint8(at + i, ch.charCodeAt(0)));
  text(0, "RIFF");
  view.setUint32(4, 36 + dataBytes, true);
  text(8, "WAVE");
  text(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, count, true);
  view.setUint32(24, rate, true);
  view.setUint32(28, rate * count * 2, true);
  view.setUint16(32, count * 2, true);
  view.setUint16(34, 16, true);
  text(36, "data");
  view.setUint32(40, dataBytes, true);
  let at = WAV_HEADER_BYTES;
  for (let i = 0; i < frames; i++) {
    for (let c = 0; c < count; c++) {
      const v = Math.max(-1, Math.min(1, channels[c][i]));
      view.setInt16(at, v < 0 ? Math.round(v * 0x8000) : Math.round(v * 0x7fff), true);
      at += 2;
    }
  }
  return out;
}

function formatTrimTime(secs) {
  const safe = Number.isFinite(secs) && secs > 0 ? secs : 0;
  const tenths = Math.floor(safe * 10 + 1e-6);
  const minutes = Math.floor(tenths / 600);
  const seconds = (tenths % 600) / 10;
  return `${minutes}:${seconds.toFixed(1).padStart(4, "0")}`;
}

function soundNameDraft(text) {
  return String(text || "").toLowerCase().replace(/[\s-]+/g, "_").replace(/[^a-z0-9_]/g, "").slice(0, 32);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    SOUND_STORE_MAX_BYTES, SOUND_SOURCE_MAX_BYTES, SOUND_SOURCE_MAX_SECONDS, SOUND_TRIM_RATE, SOUND_TRIM_MIN_SECONDS,
    isStorableSound, initialSoundSelection, moveSoundSelection, keepsOriginalSound, trimmedSoundChannels,
    soundWaveformPeaks, fadeSoundEdges, encodeWav, formatTrimTime, soundNameDraft,
  };
}
