export const PROTOCOL_VERSION = 1;
export const RELEASE_PORT = 47615;
export const TEST_BUILD_PORT = 47616;
export const COMMY_PORTS = [RELEASE_PORT, TEST_BUILD_PORT];
export const COMMY_HOST = "127.0.0.1";

export type SoundChoice = { name: string; label: string; emoji: string | null };

export type SoundboardServer = { id: string; name: string; can_play: boolean; sounds: SoundChoice[] };

export type SoundboardList = {
  voice_server: string | null;
  defaults: SoundChoice[];
  servers: SoundboardServer[];
};

export type DenyReason = "busy" | "declined" | "removed" | "unsupported";

export type Reply =
  | { type: "waiting" }
  | { type: "paired"; token: string }
  | { type: "ready"; version: number }
  | { type: "denied"; reason: DenyReason }
  | ({ type: "soundboards" } & SoundboardList)
  | { type: "played"; id: number }
  | { type: "failed"; id: number; message: string };

export type Request =
  | { type: "hello"; version: number; name: string; token?: string }
  | { type: "list" }
  | { type: "play"; id: number; server?: string; sound: string };

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
