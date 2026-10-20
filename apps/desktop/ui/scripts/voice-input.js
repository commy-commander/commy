const SILENCE_DB = -100;
const TALK_RELEASE_OPTIONS = [
  [0, "Instant"],
  [100, "100 ms"],
  [200, "200 ms"],
  [500, "500 ms"],
  [1000, "1 second"],
];
const DEFAULT_TALK_RELEASE_MS = 200;
const DEFAULT_SENSITIVITY_DB = -45;

function meterPercent(db) {
  const level = Number.isFinite(db) ? db : SILENCE_DB;
  return Math.max(0, Math.min(100, ((level - SILENCE_DB) / -SILENCE_DB) * 100));
}

function sensitivityLabel(auto, db) {
  if (auto) return "Automatic";
  return `${Math.round(Number.isFinite(db) ? db : DEFAULT_SENSITIVITY_DB)} dB`;
}

function meterState(db, auto, thresholdDb, listening) {
  if (!listening) return "idle";
  if (auto) return db > -60 ? "open" : "quiet";
  return db >= thresholdDb ? "open" : "quiet";
}

function inputModeCopy(mode) {
  if (mode === "push_to_talk") {
    return { title: "Push to Talk", text: "Your mic sends only while the talk key is held, in any app." };
  }
  return { title: "Voice Activity", text: "Your mic opens on its own when you speak." };
}

function talkKeyTitle(mode, key, talking) {
  if (mode !== "push_to_talk") return "";
  return talking ? `Talking (${key})` : `Hold ${key} to talk`;
}

function voiceInputState(voice) {
  const v = voice || {};
  return {
    mode: v.input_mode === "push_to_talk" ? "push_to_talk" : "voice_activity",
    talkKey: v.talk_key || "Ctrl+Shift+Space",
    releaseMs: Number.isInteger(v.talk_release_ms) ? v.talk_release_ms : DEFAULT_TALK_RELEASE_MS,
    muteKey: v.mute_hotkey || "",
    deafenKey: v.deafen_hotkey || "",
    auto: v.auto_sensitivity !== false,
    thresholdDb: Number.isFinite(v.sensitivity_db) ? v.sensitivity_db : DEFAULT_SENSITIVITY_DB,
  };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { SILENCE_DB, TALK_RELEASE_OPTIONS, DEFAULT_TALK_RELEASE_MS, DEFAULT_SENSITIVITY_DB, meterPercent, sensitivityLabel, meterState, inputModeCopy, talkKeyTitle, voiceInputState };
}
