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
