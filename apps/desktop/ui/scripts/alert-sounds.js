const ALERT_SOUND_PREFIX = "custom:";
const MAX_ALERT_SOUNDS = 10;
const ADD_ALERT_SOUND = "add";
const NO_SOUND = "none";
const RING_GAP_MS = 1200;

function isAlertSound(value) {
  return typeof value === "string" && value.startsWith(ALERT_SOUND_PREFIX) && value.length > ALERT_SOUND_PREFIX.length;
}

function alertSoundId(name, taken) {
  const stem = String(name || "").toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "").slice(0, 32) || "sound";
  const used = new Set(taken || []);
  let id = `${ALERT_SOUND_PREFIX}${stem}`;
  for (let n = 2; used.has(id); n++) id = `${ALERT_SOUND_PREFIX}${stem}_${n}`;
  return id;
}

function builtInSoundLabel(name) {
  return name[0].toUpperCase() + name.slice(1);
}

function alertSoundChoices(builtIns, custom, canAdd) {
  const groups = [{ label: "Built in", options: builtIns.map(name => ({ value: name, label: builtInSoundLabel(name) })) }];
  if (custom.length) groups.push({ label: "Your sounds", options: custom.map(s => ({ value: s.id, label: s.name })) });
  const more = [];
  if (canAdd) more.push({ value: ADD_ALERT_SOUND, label: "Add your own..." });
  more.push({ value: NO_SOUND, label: "No sound" });
  groups.push({ label: "", options: more });
  return groups;
}

function resolveAlertSound(chosen, fallback, custom) {
  if (chosen === undefined || chosen === null) return fallback;
  if (isAlertSound(chosen) && !custom.some(s => s.id === chosen)) return fallback;
  return chosen;
}

function joinSoundToPlay({ joined, joinSounds, allowed, hearJoinSounds }) {
  if (!allowed || !hearJoinSounds || joined.length !== 1) return null;
  return (joinSounds || {})[joined[0]] || null;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    ALERT_SOUND_PREFIX, MAX_ALERT_SOUNDS, ADD_ALERT_SOUND, NO_SOUND, RING_GAP_MS,
    isAlertSound, alertSoundId, builtInSoundLabel, alertSoundChoices, resolveAlertSound, joinSoundToPlay,
  };
}
