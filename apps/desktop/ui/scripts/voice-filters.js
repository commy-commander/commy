"use strict";

const VOICE_FILTERS = [
  { key: "off", get label() { return t("ui.off"); }, get sub() { return t("ui.your-own-voice"); } },
  { key: "higher", get label() { return t("ui.higher"); }, get sub() { return t("ui.bright-and-squeaky"); } },
  { key: "lower", get label() { return t("ui.lower"); }, get sub() { return t("ui.deep-and-booming"); } },
  { key: "robot", get label() { return t("ui.robot"); }, get sub() { return t("ui.one-steady-metal-note"); } },
  { key: "radio", get label() { return t("ui.radio"); }, get sub() { return t("ui.crackly-walkie-talkie"); } },
  { key: "cave", get label() { return t("ui.cave"); }, get sub() { return t("ui.echoes-off-stone-walls"); } },
  { key: "whisper", get label() { return t("ui.whisper"); }, get sub() { return t("ui.breathy-and-hushed"); } },
];

function voiceFilterKey(voice) {
  const key = voice && voice.voice_filter;
  return VOICE_FILTERS.some(f => f.key === key) ? key : "off";
}

function voiceFilterLabel(key) {
  const found = VOICE_FILTERS.find(f => f.key === key);
  return found ? found.label : t("ui.off");
}

function voiceFilterHeldHere(held, voiceServer) {
  return Boolean(held && held.held && voiceServer && voiceServer.id && held.server === voiceServer.id);
}

function voiceFilterNote(key, heldHere, serverName) {
  if (key === "off") return t("ui.everyone-hears-your-own-voice");
  if (heldHere) return t("ui.has-voice-filters-switched-off-so", { value: serverName || t("ui.this-server") });
  return t("ui.everyone-in-voice-hears-you-through-the", { value: voiceFilterLabel(key) });
}

function voiceFilterNews(held, serverName) {
  if (!held) return t("ui.your-voice-filter-is-back-on");
  return t("ui.voice-filters-are-off-in-so-everyone", { value: serverName || "this server" });
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { VOICE_FILTERS, voiceFilterKey, voiceFilterLabel, voiceFilterHeldHere, voiceFilterNote, voiceFilterNews };
}
