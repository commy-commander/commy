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
const BUILT_IN_VOICE_PREFIX = "piper:";
const SYSTEM_VOICE_PREFIX = "system:";
const DEFAULT_BUILT_IN_VOICE = "en_US-kristin-medium";
const SPEECH_CHUNK_CHARS = 400;

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

function langParts(lang) {
  const [language = "", region = ""] = String(lang || "").toLowerCase().replace("_", "-").split("-");
  return { language, region };
}

function defaultBuiltInVoice(builtIn, lang) {
  if (!builtIn.length) return null;
  const want = langParts(lang);
  const fallback = builtIn.find(v => v.id === DEFAULT_BUILT_IN_VOICE) || builtIn[0];
  const sameLanguage = builtIn.filter(v => langParts(v.lang).language === want.language);
  if (!sameLanguage.length) return fallback.id;
  const sameRegion = sameLanguage.filter(v => langParts(v.lang).region === want.region);
  const pool = sameRegion.length ? sameRegion : sameLanguage;
  return (pool.includes(fallback) ? fallback : pool[0]).id;
}

function resolveTtsVoice(pref, builtIn, system, lang) {
  const saved = String(pref || "");
  const builtInList = Array.isArray(builtIn) ? builtIn : [];
  const systemList = Array.isArray(system) ? system : [];
  if (saved.startsWith(BUILT_IN_VOICE_PREFIX)) {
    const id = saved.slice(BUILT_IN_VOICE_PREFIX.length);
    if (builtInList.some(v => v.id === id)) return { kind: "builtin", id };
  } else if (saved === SYSTEM_VOICE_PREFIX) {
    return { kind: "system", id: null };
  } else if (saved) {
    const id = saved.startsWith(SYSTEM_VOICE_PREFIX) ? saved.slice(SYSTEM_VOICE_PREFIX.length) : saved;
    if (systemList.some(v => v.id === id)) return { kind: "system", id };
  }
  const fallback = defaultBuiltInVoice(builtInList, lang);
  return fallback ? { kind: "builtin", id: fallback } : { kind: "system", id: null };
}

function ttsVoiceValue(choice) {
  if (choice.kind === "builtin") return BUILT_IN_VOICE_PREFIX + choice.id;
  return SYSTEM_VOICE_PREFIX + (choice.id || "");
}

function megabytes(bytes) {
  return `${Math.max(1, Math.round(Number(bytes || 0) / 1e6))} MB`;
}

function ttsVoiceStatus(choice, voice, progress) {
  if (choice.kind !== "builtin" || !voice) return "Uses this device's own voices.";
  if (voice.ready) return "Built into Commy, works offline.";
  if (progress && progress.total > 0) return `Downloading, ${Math.min(99, Math.floor((progress.loaded / progress.total) * 100))}%`;
  return `Downloads ${megabytes(voice.download_bytes)} the first time it speaks.`;
}

function piperTiming(inference, rate, pitch) {
  const base = inference || {};
  const speed = clampRate(rate);
  const tone = clampPitch(pitch);
  const lengthScale = (Number(base.length_scale) || 1) * (tone / speed);
  return { scales: [Number(base.noise_scale ?? 0.667), lengthScale, Number(base.noise_w ?? 0.8)], playbackRate: tone };
}

function speechChunks(text, max = SPEECH_CHUNK_CHARS) {
  const clean = String(text || "").replace(/\s+/g, " ").trim();
  if (!clean) return [];
  const sentences = clean.match(/[^.!?…]+[.!?…]*\s*/g) || [clean];
  const chunks = [];
  let current = "";
  const push = piece => { if (piece.trim()) chunks.push(piece.trim()); };
  for (const sentence of sentences) {
    if (current && (current + sentence).length > max) {
      push(current);
      current = "";
    }
    if (sentence.length <= max) {
      current += sentence;
      continue;
    }
    for (const word of sentence.split(" ")) {
      if (current && (current + " " + word).length > max) {
        push(current);
        current = "";
      }
      current = current ? `${current} ${word}` : word.slice(0, max);
    }
  }
  push(current);
  return chunks;
}

function shouldSpeakIncoming({ mode, tts, deafened, viewing, blocked, presence, inVoice }) {
  if (!tts || deafened || blocked || !inVoice) return false;
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
    BUILT_IN_VOICE_PREFIX, SYSTEM_VOICE_PREFIX, DEFAULT_BUILT_IN_VOICE, SPEECH_CHUNK_CHARS,
    defaultBuiltInVoice, resolveTtsVoice, ttsVoiceValue, ttsVoiceStatus, piperTiming, speechChunks, megabytes,
  };
}
