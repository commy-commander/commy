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
  const groups = [{ label: t("ui.built-in"), options: builtIns.map(name => ({ value: name, label: builtInSoundLabel(name) })) }];
  if (custom.length) groups.push({ label: t("ui.your-sounds-2"), options: custom.map(s => ({ value: s.id, label: s.name })) });
  const more = [];
  if (canAdd) more.push({ value: ADD_ALERT_SOUND, label: t("ui.add-your-own") });
  more.push({ value: NO_SOUND, label: t("ui.no-sound") });
  groups.push({ label: "", options: more });
  return groups;
}

function resolveAlertSound(chosen, fallback, custom) {
  if (chosen === undefined || chosen === null) return fallback;
  if (isAlertSound(chosen) && !custom.some(s => s.id === chosen)) return fallback;
  return chosen;
}

function builtInSoundFile(name, files, studioFiles, studio) {
  return (studio && studioFiles[name]) || files[name] || null;
}

function joinSoundToPlay({ joined, joinSounds, allowed, hearJoinSounds }) {
  if (!allowed || !hearJoinSounds || joined.length !== 1) return null;
  return (joinSounds || {})[joined[0]] || null;
}

const LEAVE_CUE_HOLD_MS = 2500;

function voiceRosterChange(before, now) {
  const joined = [...now].filter(id => !before.has(id));
  const gone = [...before].filter(id => !now.has(id));
  return { joined, left: gone.length > 0, gone };
}

function cuesAfterHold(change, held) {
  return {
    joined: change.joined.filter(id => !held.has(id)),
    back: change.joined.filter(id => held.has(id)),
    gone: change.gone,
  };
}

function monoSamples(channels) {
  const list = Array.from(channels || []);
  if (!list.length) return new Float32Array(0);
  const length = Math.min(...list.map(c => c.length));
  const out = new Float32Array(length);
  for (const channel of list) for (let i = 0; i < length; i++) out[i] += channel[i] / list.length;
  return out;
}

function soundLevel(volume) {
  const n = Number(volume);
  return Number.isFinite(n) ? Math.max(0, Math.min(1, n)) : 1;
}

function playAlone(playing, key, start) {
  playing.get(key)?.stop();
  const sound = start();
  if (!sound) {
    playing.delete(key);
    return sound;
  }
  playing.set(key, sound);
  sound.ended.then(() => {
    if (playing.get(key) === sound) playing.delete(key);
  });
  return sound;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    ALERT_SOUND_PREFIX, MAX_ALERT_SOUNDS, ADD_ALERT_SOUND, NO_SOUND, RING_GAP_MS,
    isAlertSound, alertSoundId, builtInSoundLabel, alertSoundChoices, resolveAlertSound, builtInSoundFile, joinSoundToPlay,
    LEAVE_CUE_HOLD_MS, voiceRosterChange, cuesAfterHold, monoSamples, soundLevel, playAlone,
  };
}
