export const PROTOCOL_VERSION = 1;
export const RELEASE_PORT = 47615;
export const TEST_BUILD_PORT = 47616;
export const COMMY_PORTS = [RELEASE_PORT, TEST_BUILD_PORT];
export const COMMY_HOST = "127.0.0.1";

export type SoundChoice = { name: string; label: string; emoji: string | null };

export type SoundboardServer = { id: string; name: string; can_play: boolean; can_add: boolean; sounds: SoundChoice[] };

export type SoundboardList = {
  voice_server: string | null;
  defaults: SoundChoice[];
  servers: SoundboardServer[];
};

export type VolumeTarget = "output" | "input";

export type Change = { by: number } | { to: number };

export type Control =
  | { action: "toggle_mute" }
  | { action: "toggle_deafen" }
  | { action: "talk"; down: boolean }
  | { action: "hold_mute"; down: boolean }
  | { action: "toggle_input_mode" }
  | { action: "toggle_camera" }
  | { action: "toggle_stream" }
  | { action: "volume"; target: VolumeTarget; change: Change }
  | { action: "user_volume"; user: string; change: Change }
  | { action: "device"; target: VolumeTarget; device: string }
  | { action: "join_voice"; server: string; channel: string }
  | { action: "open_channel"; server: string; channel: string }
  | { action: "open_app" };

export type ServerStats = { id: string; members: number; online: number; in_voice: number };

export type DeckStatus = {
  muted: boolean;
  deafened: boolean;
  push_to_talk: boolean;
  voice: { server: string; channel: string } | null;
  camera: boolean;
  sharing: boolean;
  output_volume: number;
  input_gain: number;
  input_device: string | null;
  output_device: string | null;
  user_volumes: { user: string; volume: number }[];
  unread: number;
  servers: ServerStats[];
};

export type ChannelChoice = { id: string; name: string };

export type Directory = {
  servers: { id: string; name: string; text: ChannelChoice[]; voice: ChannelChoice[] }[];
  people: { id: string; name: string }[];
  inputs: string[];
  outputs: string[];
};

export type DenyReason = "busy" | "declined" | "removed" | "unsupported";

export type Reply =
  | { type: "waiting" }
  | { type: "paired"; token: string }
  | { type: "ready"; version: number }
  | { type: "denied"; reason: DenyReason }
  | ({ type: "soundboards" } & SoundboardList)
  | { type: "played"; id: number }
  | { type: "added"; id: number; server: string; sound: SoundChoice }
  | ({ type: "status" } & DeckStatus)
  | ({ type: "directory" } & Directory)
  | { type: "done"; id: number; status: DeckStatus }
  | { type: "failed"; id: number; message: string };

export type Request =
  | { type: "hello"; version: number; name: string; token?: string }
  | { type: "list" }
  | { type: "play"; id: number; server?: string; sound: string }
  | { type: "add_sound"; id: number; server: string; name: string; emoji?: string; data: string }
  | { type: "status" }
  | { type: "directory" }
  | { type: "control"; id: number; control: Control };

export function parseReply(line: string): Reply | null {
  try {
    const value = JSON.parse(line);
    return value && typeof value.type === "string" ? (value as Reply) : null;
  } catch {
    return null;
  }
}

export function encodeRequest(request: Request): string {
  return `${JSON.stringify(request)}\n`;
}
