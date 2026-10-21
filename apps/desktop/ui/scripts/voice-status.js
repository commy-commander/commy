const VOICE_STATUS_MAX_CHARS = 80;
const PRIORITY_DUCK_GAIN = 0.3;

function cleanVoiceStatus(text) {
  const tidy = String(text || "")
    .replace(/[\u0000-\u001f\u007f-\u009f]/g, "")
    .split(/\s+/)
    .filter(Boolean)
    .join(" ");
  return [...tidy].slice(0, VOICE_STATUS_MAX_CHARS).join("").trimEnd();
}

function voiceStatusProblem(text) {
  const length = [...String(text || "").trim()].length;
  return length > VOICE_STATUS_MAX_CHARS ? `Keep the status to ${VOICE_STATUS_MAX_CHARS} characters` : null;
}

function canSetVoiceStatus({ channel, server, me }) {
  if (!channel || channel.kind !== "voice") return false;
  if (typeof channel.can_set_voice_status === "boolean") return channel.can_set_voice_status;
  if (server && server.can_manage_channels) return true;
  return Boolean(me) && (channel.voice_user_ids || []).includes(me);
}

function prioritySpeakersIn(channel, members) {
  const holders = new Set((members || []).filter(m => m.priority_speaker).map(m => m.id));
  return new Set((channel && channel.voice_user_ids ? channel.voice_user_ids : []).filter(id => holders.has(id)));
}

function priorityTalking(priority, speaking) {
  for (const id of priority || []) if (speaking && speaking.has(id)) return true;
  return false;
}

function attenuationGain(volume, { isPriority = false, ducked = false } = {}) {
  const base = Number.isFinite(volume) ? Math.max(0, volume) : 1;
  return ducked && !isPriority ? base * PRIORITY_DUCK_GAIN : base;
}

function attenuatedUsers(channel, members, speaking) {
  const priority = prioritySpeakersIn(channel, members);
  if (!priorityTalking(priority, speaking)) return new Set();
  return new Set((channel.voice_user_ids || []).filter(id => !priority.has(id)));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    VOICE_STATUS_MAX_CHARS,
    PRIORITY_DUCK_GAIN,
    cleanVoiceStatus,
    voiceStatusProblem,
    canSetVoiceStatus,
    prioritySpeakersIn,
    priorityTalking,
    attenuationGain,
    attenuatedUsers,
  };
}
