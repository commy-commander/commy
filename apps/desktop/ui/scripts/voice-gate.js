const VOICE_GATE_OPEN = { enabled: false, waits: {} };

function readVoiceGate(view) {
  if (!view || typeof view !== "object") return VOICE_GATE_OPEN;
  return { enabled: view.enabled === true, waits: view.waits && typeof view.waits === "object" ? view.waits : {} };
}

function voiceGateWait(gate, serverId, voiceServerId) {
  if (!gate || !gate.enabled || !serverId) return null;
  if (voiceServerId && voiceServerId === serverId) return null;
  const wait = gate.waits[serverId];
  return wait && wait.reason ? wait : null;
}

function voiceGateRefusal(wait) {
  return `Voice opens once you are connected. ${wait.reason}.`;
}

function sameVoiceGate(a, b) {
  if (a.enabled !== b.enabled) return false;
  const keys = Object.keys(a.waits);
  if (keys.length !== Object.keys(b.waits).length) return false;
  return keys.every(key => b.waits[key] && b.waits[key].kind === a.waits[key].kind);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { VOICE_GATE_OPEN, readVoiceGate, voiceGateWait, voiceGateRefusal, sameVoiceGate };
}
