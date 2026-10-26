import type { Change, Control, DeckStatus, VolumeTarget } from "./protocol.ts";
import { wrapTitle } from "./sound-keys.ts";

export const CONTROL_KINDS = [
  "mute",
  "deafen",
  "push-to-talk",
  "push-to-mute",
  "input-mode",
  "camera",
  "stream",
  "voice-channel",
  "text-channel",
  "volume",
  "user-volume",
  "audio-device",
  "server-stats",
  "notifications",
] as const;

export type ControlKind = (typeof CONTROL_KINDS)[number];

export type VolumeMode = "up" | "down" | "set";

export type ControlSettings = {
  server?: string;
  channel?: string;
  label?: string;
  target?: VolumeTarget;
  mode?: VolumeMode;
  amount?: number;
  user?: string;
  device?: string;
};

export type KeyLook = { state: 0 | 1; title: string };

export const DEFAULT_STEP_PERCENT = 10;
export const DIAL_STEP_PERCENT = 2;

export function actionId(kind: ControlKind): string {
  return `chat.commy.soundboard.${kind}`;
}

function percent(value: number): string {
  return `${Math.round(value * 100)}%`;
}

function volumeChange(settings: ControlSettings): Change {
  const amount = Number.isFinite(settings.amount) ? Number(settings.amount) : DEFAULT_STEP_PERCENT;
  const fraction = Math.max(0, Math.min(200, amount)) / 100;
  if (settings.mode === "set") return { to: fraction };
  return { by: settings.mode === "down" ? -fraction : fraction };
}

export function pressControl(kind: ControlKind, settings: ControlSettings): Control | null {
  switch (kind) {
    case "mute":
      return { action: "toggle_mute" };
    case "deafen":
      return { action: "toggle_deafen" };
    case "push-to-talk":
      return { action: "talk", down: true };
    case "push-to-mute":
      return { action: "hold_mute", down: true };
    case "input-mode":
      return { action: "toggle_input_mode" };
    case "camera":
      return { action: "toggle_camera" };
    case "stream":
      return { action: "toggle_stream" };
    case "voice-channel":
      return settings.server && settings.channel ? { action: "join_voice", server: settings.server, channel: settings.channel } : null;
    case "text-channel":
      return settings.server && settings.channel ? { action: "open_channel", server: settings.server, channel: settings.channel } : null;
    case "volume":
      return { action: "volume", target: settings.target ?? "output", change: volumeChange(settings) };
    case "user-volume":
      return settings.user ? { action: "user_volume", user: settings.user, change: volumeChange(settings) } : null;
    case "audio-device":
      return settings.device !== undefined ? { action: "device", target: settings.target ?? "output", device: settings.device } : null;
    case "server-stats":
    case "notifications":
      return { action: "open_app" };
  }
}

export function releaseControl(kind: ControlKind): Control | null {
  if (kind === "push-to-talk") return { action: "talk", down: false };
  if (kind === "push-to-mute") return { action: "hold_mute", down: false };
  return null;
}

export function dialControl(kind: ControlKind, settings: ControlSettings, ticks: number): Control | null {
  const change = { by: (ticks * DIAL_STEP_PERCENT) / 100 };
  if (kind === "volume") return { action: "volume", target: settings.target ?? "output", change };
  if (kind === "user-volume" && settings.user) return { action: "user_volume", user: settings.user, change };
  return null;
}

function titled(text: string): string {
  return wrapTitle(text).join("\n");
}

export function volumeOf(kind: ControlKind, settings: ControlSettings, status: DeckStatus): number | null {
  if (kind === "volume") return settings.target === "input" ? status.input_gain : status.output_volume;
  if (kind === "user-volume" && settings.user) return status.user_volumes.find(v => v.user === settings.user)?.volume ?? 1;
  return null;
}

export function keyLook(kind: ControlKind, settings: ControlSettings, status: DeckStatus | null): KeyLook {
  const off: KeyLook = { state: 0, title: "" };
  if (!status) {
    if (kind === "voice-channel" || kind === "text-channel" || kind === "user-volume" || kind === "audio-device") {
      return { state: 0, title: settings.label ? titled(settings.label) : "" };
    }
    return off;
  }
  switch (kind) {
    case "mute":
      return { state: status.muted ? 1 : 0, title: "" };
    case "deafen":
      return { state: status.deafened ? 1 : 0, title: "" };
    case "push-to-talk":
      return { state: 0, title: status.push_to_talk ? "" : "Off" };
    case "push-to-mute":
      return off;
    case "input-mode":
      return { state: status.push_to_talk ? 1 : 0, title: status.push_to_talk ? "Push to\ntalk" : "Voice\nactivity" };
    case "camera":
      return { state: status.camera ? 1 : 0, title: "" };
    case "stream":
      return { state: status.sharing ? 1 : 0, title: "" };
    case "voice-channel": {
      const here = status.voice?.server === settings.server && status.voice?.channel === settings.channel;
      return { state: here ? 1 : 0, title: settings.label ? titled(settings.label) : "" };
    }
    case "text-channel":
      return { state: 0, title: settings.label ? titled(`#${settings.label}`) : "" };
    case "volume": {
      const name = settings.target === "input" ? "Mic" : "Volume";
      return { state: 0, title: `${name}\n${percent(volumeOf(kind, settings, status) ?? 1)}` };
    }
    case "user-volume": {
      if (!settings.user) return off;
      const name = wrapTitle(settings.label || "Someone", 9, 1)[0];
      return { state: 0, title: `${name}\n${percent(volumeOf(kind, settings, status) ?? 1)}` };
    }
    case "audio-device": {
      const current = settings.target === "input" ? status.input_device : status.output_device;
      const chosen = settings.device || null;
      return { state: current === chosen ? 1 : 0, title: settings.label ? titled(settings.label) : "" };
    }
    case "server-stats": {
      const stats = status.servers.find(s => s.id === settings.server);
      if (!stats) return off;
      const name = settings.label ? [wrapTitle(settings.label, 9, 1)[0]] : [];
      return { state: 0, title: [...name, `${stats.online}/${stats.members}`, "online"].join("\n") };
    }
    case "notifications":
      return { state: status.unread > 0 ? 1 : 0, title: status.unread > 0 ? String(Math.min(status.unread, 999)) : "" };
  }
}
