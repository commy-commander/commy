const SILENCE_DB = -100;
const TALK_RELEASE_OPTIONS = [
  [0, t("ui.instant")],
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
  if (auto) return t("ui.automatic");
  return t("ui.db", { value: Math.round(Number.isFinite(db) ? db : DEFAULT_SENSITIVITY_DB) });
}

function meterState(db, auto, thresholdDb, listening) {
  if (!listening) return "idle";
  if (auto) return db > -60 ? "open" : "quiet";
  return db >= thresholdDb ? "open" : "quiet";
}

function inputModeCopy(mode, onPhone = false, inBrowser = false) {
  if (mode === "push_to_talk") {
    const text = onPhone ? t("ui.your-mic-sends-only-while-you-hold-the") : inBrowser ? t("ui.your-mic-sends-only-while-the-talk-key") : t("ui.your-mic-sends-only-while-the-talk-key-2");
    return { title: t("ui.push-to-talk"), text };
  }
  return { title: t("ui.voice-activity"), text: t("ui.your-mic-opens-on-its-own-when-you-speak") };
}

function talkKeyTitle(mode, key, talking) {
  if (mode !== "push_to_talk") return "";
  return talking ? t("ui.talking-2", { key }) : t("ui.hold-to-talk-2", { key });
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
