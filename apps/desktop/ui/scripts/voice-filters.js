"use strict";

const VOICE_FILTERS = [
  { key: "off", label: "Off", sub: "Your own voice" },
  { key: "higher", label: "Higher", sub: "Bright and squeaky" },
  { key: "lower", label: "Lower", sub: "Deep and booming" },
  { key: "robot", label: "Robot", sub: "One steady metal note" },
  { key: "radio", label: "Radio", sub: "Crackly walkie-talkie" },
  { key: "cave", label: "Cave", sub: "Echoes off stone walls" },
  { key: "whisper", label: "Whisper", sub: "Breathy and hushed" },
];

function voiceFilterKey(voice) {
  const key = voice && voice.voice_filter;
  return VOICE_FILTERS.some(f => f.key === key) ? key : "off";
}

function voiceFilterLabel(key) {
  const found = VOICE_FILTERS.find(f => f.key === key);
  return found ? found.label : "Off";
}

function voiceFilterHeldHere(held, voiceServer) {
  return Boolean(held && held.held && voiceServer && voiceServer.id && held.server === voiceServer.id);
}

function voiceFilterNote(key, heldHere, serverName) {
  if (key === "off") return "Everyone hears your own voice.";
  if (heldHere) return `${serverName || "This server"} has voice filters switched off, so everyone there hears your own voice.`;
  return `Everyone in voice hears you through the ${voiceFilterLabel(key)} filter. Start the mic test to hear it yourself.`;
}

function voiceFilterNews(held, serverName) {
  if (!held) return "Your voice filter is back on";
  return `Voice filters are off in ${serverName || "this server"}, so everyone hears your own voice`;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { VOICE_FILTERS, voiceFilterKey, voiceFilterLabel, voiceFilterHeldHere, voiceFilterNote, voiceFilterNews };
}
