import type { SoundboardList } from "./protocol.ts";
import type { LinkState } from "./commy-link.ts";

export const VOICE_SERVER = "";
export const TITLE_LINE_CHARS = 9;
export const TITLE_MAX_LINES = 3;

export type SoundKeySettings = {
  server?: string;
  sound?: string;
  label?: string;
  emoji?: string | null;
};

export function wrapTitle(text: string, width = TITLE_LINE_CHARS, maxLines = TITLE_MAX_LINES): string[] {
  const lines: string[] = [];
  let current = "";
  for (const word of text.trim().split(/\s+/).filter(Boolean)) {
    for (const piece of word.match(new RegExp(`.{1,${width}}`, "gu")) ?? []) {
      if (!current) current = piece;
      else if (current.length + 1 + piece.length <= width) current += ` ${piece}`;
      else {
        lines.push(current);
        current = piece;
      }
    }
  }
  if (current) lines.push(current);
  if (lines.length <= maxLines) return lines;
  const kept = lines.slice(0, maxLines);
  kept[maxLines - 1] = `${kept[maxLines - 1].slice(0, width - 1)}…`;
  return kept;
}

export function keyTitle(settings: SoundKeySettings): string {
  if (!settings.sound) return "";
  const lines = wrapTitle(settings.label || settings.sound);
  return (settings.emoji ? [settings.emoji, ...lines] : lines).join("\n");
}

export function playTarget(settings: SoundKeySettings): { sound: string; server?: string } | null {
  if (!settings.sound) return null;
  return settings.server ? { sound: settings.sound, server: settings.server } : { sound: settings.sound };
}

export function chosenSound(list: SoundboardList, settings: SoundKeySettings): SoundKeySettings | null {
  if (!settings.sound) return null;
  const pool = settings.server
    ? [...list.defaults, ...(list.servers.find(s => s.id === settings.server)?.sounds ?? [])]
    : list.defaults;
  const choice = pool.find(s => s.name === settings.sound);
  return choice ? { ...settings, label: choice.label, emoji: choice.emoji } : null;
}

const STATE_MESSAGES: Record<LinkState, string> = {
  offline: "Open Commy and turn on Stream Deck in Settings > Integrations.",
  waiting: "Choose Allow in Commy to finish connecting.",
  declined: "Commy did not allow this Stream Deck. Open these settings again to ask once more.",
  busy: "Commy is already asking about another Stream Deck. Answer that first.",
  unsupported: "Update Commy to use this plugin.",
  ready: "",
};

export function stateMessage(state: LinkState): string {
  return STATE_MESSAGES[state];
}
