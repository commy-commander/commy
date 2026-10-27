"use strict";

const TTS_COMMAND = "tts";
const TTS_MARKER_MIME = "application/vnd.commy.tts";
const TTS_MODES = [
  { id: "never", label: "Never" },
  { id: "open", label: "Only when the channel is open" },
  { id: "all", label: "All channels" },
];
const DEFAULT_TTS_MODE = "open";
const TTS_RATE_MIN = 0.5;
const TTS_RATE_MAX = 2;
const DEFAULT_TTS_RATE = 1;
const TTS_PITCH_MIN = 0.5;
const TTS_PITCH_MAX = 2;
const DEFAULT_TTS_PITCH = 1;
const TTS_MAX_SPOKEN_CHARS = 500;

function parseTtsPrefix(text) {
  const raw = String(text || "");
  const match = raw.match(/^\/tts(?=\s|$)\s*([^]*)$/i);
  if (!match) return { tts: false, content: raw.trim() };
  return { tts: true, content: match[1].trim() };
}

function canSendTts(server) {
  return !server || server.can_send_tts_messages !== false;
}

function clampTenths(value, min, max, fallback) {
  if (value === null || value === undefined || value === "") return fallback;
  const n = Number(value);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(max, Math.max(min, Math.round(n * 10) / 10));
}

function clampRate(value) {
  return clampTenths(value, TTS_RATE_MIN, TTS_RATE_MAX, DEFAULT_TTS_RATE);
}

function clampPitch(value) {
  return clampTenths(value, TTS_PITCH_MIN, TTS_PITCH_MAX, DEFAULT_TTS_PITCH);
}

function readTtsPrefs(raw) {
  let parsed = null;
  try {
    parsed = raw ? JSON.parse(raw) : null;
  } catch {
    parsed = null;
  }
  const mode = parsed && TTS_MODES.some(m => m.id === parsed.mode) ? parsed.mode : DEFAULT_TTS_MODE;
  const rate = clampRate(parsed && parsed.rate);
  const pitch = clampPitch(parsed && parsed.pitch);
  const voice = parsed && typeof parsed.voice === "string" && parsed.voice ? parsed.voice : null;
  return { mode, rate, pitch, voice };
}

function ttsVoices(raw, preferredLang) {
  const seen = new Set();
  const voices = [];
  for (const v of Array.isArray(raw) ? raw : []) {
    const id = v && String(v.id || "");
    if (!id || seen.has(id)) continue;
    seen.add(id);
    voices.push({ id, name: String(v.name || id), lang: String(v.lang || ""), default: Boolean(v.default) });
  }
  const want = String(preferredLang || "").toLowerCase().split(/[-_]/)[0];
  const rank = v => (want && v.lang.toLowerCase().replace("_", "-").split("-")[0] === want ? 0 : 1);
  return voices.sort((a, b) => rank(a) - rank(b) || a.lang.localeCompare(b.lang) || a.name.localeCompare(b.name));
}

function ttsVoiceLabel(voice) {
  const name = voice.name.replace(/^Microsoft\s+/, "").replace(/\s+-\s+.*$/, "").trim() || voice.name;
  return voice.lang ? `${name} (${voice.lang})` : name;
}

function chosenTtsVoice(voices, id) {
  return (id && voices.find(v => v.id === id)) || null;
}

function shouldSpeakIncoming({ mode, tts, ownMessage, deafened, viewing, blocked, presence }) {
  if (!tts || ownMessage || deafened || blocked) return false;
  if (presence === "do_not_disturb") return false;
  if (mode === "all") return true;
  return mode === "open" && Boolean(viewing);
}

function spokenText(authorName, content, plain) {
  const text = (plain ? plain(content || "") : String(content || "")).replace(/\s+/g, " ").trim();
  if (!text) return null;
  const clipped = text.length > TTS_MAX_SPOKEN_CHARS ? `${text.slice(0, TTS_MAX_SPOKEN_CHARS).trim()}, and more` : text;
  return `${authorName || "Someone"} said: ${clipped}`;
}

function ttsCommandHint(item) {
  if (!item.available) return "You can't send text-to-speech messages here";
  return item.args ? "Send and read aloud" : "Read a message aloud for everyone";
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    TTS_COMMAND, TTS_MARKER_MIME, TTS_MODES, DEFAULT_TTS_MODE, TTS_RATE_MIN, TTS_RATE_MAX, DEFAULT_TTS_RATE, TTS_MAX_SPOKEN_CHARS,
    TTS_PITCH_MIN, TTS_PITCH_MAX, DEFAULT_TTS_PITCH,
    parseTtsPrefix, canSendTts, clampRate, clampPitch, readTtsPrefs, shouldSpeakIncoming, spokenText, ttsCommandHint,
    ttsVoices, ttsVoiceLabel, chosenTtsVoice,
  };
}
