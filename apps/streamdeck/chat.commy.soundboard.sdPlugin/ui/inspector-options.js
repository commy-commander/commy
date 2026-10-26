export const VOICE_SERVER = "";

export function serverOptions(list, selected) {
  const options = [{ value: VOICE_SERVER, label: "The server you are in voice in", disabled: false }];
  for (const server of list.servers) {
    options.push({
      value: server.id,
      label: server.can_play ? server.name : `${server.name} (no soundboard access)`,
      disabled: !server.can_play,
    });
  }
  if (selected && !list.servers.some(s => s.id === selected)) {
    options.push({ value: selected, label: "A server you have left", disabled: true });
  }
  return options;
}

export function soundGroups(list, server) {
  const chosen = server ? list.servers.find(s => s.id === server) : null;
  const custom = chosen && chosen.sounds.length ? [{ label: chosen.name, sounds: chosen.sounds }] : [];
  return [...custom, { label: "Built in", sounds: list.defaults }];
}

export function findSound(groups, name) {
  for (const group of groups) {
    const sound = group.sounds.find(s => s.name === name);
    if (sound) return sound;
  }
  return null;
}

export function settingsFor(server, sound) {
  return sound
    ? { server, sound: sound.name, label: sound.label, emoji: sound.emoji ?? null }
    : { server };
}

export const UPLOAD_PART_CHARS = 64 * 1024;

export function uploadServers(list) {
  const open = list.servers.filter(s => s.can_add);
  const voice = open.filter(s => s.id === list.voice_server);
  return [...voice, ...open.filter(s => s.id !== list.voice_server)].map(s => ({ value: s.id, label: s.name }));
}

export function preferredUploadServer(options, settings) {
  const ids = options.map(o => o.value);
  return ids.includes(settings.server) ? settings.server : (ids[0] ?? "");
}

export function uploadParts(data, size = UPLOAD_PART_CHARS) {
  const parts = [];
  for (let at = 0; at < data.length; at += size) parts.push(data.slice(at, at + size));
  return parts.length ? parts : [""];
}

export function fileStem(fileName) {
  const base = String(fileName || "").split(/[\\/]/).pop() ?? "";
  const dot = base.lastIndexOf(".");
  return dot > 0 ? base.slice(0, dot) : base;
}

export function soundEmoji(text) {
  const trimmed = String(text || "").trim();
  return trimmed ? trimmed : null;
}
