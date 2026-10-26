export const FIELDS = {
  "voice-channel": ["server", "channel"],
  "text-channel": ["server", "channel"],
  volume: ["target", "mode", "amount"],
  "user-volume": ["user", "mode", "amount"],
  "audio-device": ["target", "device"],
  "server-stats": ["server"],
};

export const HINTS = {
  mute: "Mutes or unmutes your microphone.",
  deafen: "Deafens or undeafens you.",
  "push-to-talk": "Hold the key to talk. Turn on push to talk in Commy's voice settings, or use a Voice input mode key.",
  "push-to-mute": "Hold the key to mute your microphone, and let go to unmute.",
  "input-mode": "Switches between voice activity and push to talk.",
  camera: "Turns your camera on or off while you are in a voice channel.",
  stream: "Stops sharing, or opens Commy so you can pick a screen or game to share.",
  "voice-channel": "Press to join this channel, and again to leave it.",
  "text-channel": "Opens this channel in Commy.",
  volume: "On a dial, turn to change it.",
  "user-volume": "People in your voice channel, and people whose volume you changed, are listed. On a dial, turn to change it.",
  "audio-device": "The key lights up while this device is in use.",
  "server-stats": "Shows online members out of all members. Press to open Commy.",
  notifications: "Shows how many unread messages you have. Press to open Commy.",
};

export function kindOf(actionUuid) {
  return String(actionUuid || "").replace(/^chat\.commy\.soundboard\./, "");
}

export function withSaved(options, saved, savedLabel) {
  if (!saved || options.some(o => o.value === saved)) return options;
  return [...options, { value: saved, label: savedLabel ? `${savedLabel} (not found)` : "Not found", disabled: true }];
}

export function serverChoices(directory) {
  return directory.servers.map(s => ({ value: s.id, label: s.name }));
}

export function channelChoices(directory, server, kind) {
  const found = directory.servers.find(s => s.id === server);
  const list = found ? (kind === "voice-channel" ? found.voice : found.text) : [];
  return list.map(c => ({ value: c.id, label: kind === "voice-channel" ? c.name : `#${c.name}` }));
}

export function personChoices(directory) {
  return directory.people.map(p => ({ value: p.id, label: p.name }));
}

export function deviceChoices(directory, target) {
  const names = target === "input" ? directory.inputs : directory.outputs;
  return [{ value: "", label: "System default" }, ...names.map(n => ({ value: n, label: n }))];
}

export function labelFor(options, value) {
  return options.find(o => o.value === value && !o.disabled)?.label;
}

export function cleanAmount(raw) {
  const value = Math.round(Number(raw));
  return Number.isFinite(value) ? Math.max(1, Math.min(200, value)) : 10;
}
