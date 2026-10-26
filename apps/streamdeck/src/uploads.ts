import type { NewSound } from "./commy-link.ts";

export const MAX_SOUND_BYTES = 1024 * 1024;
export const MAX_UPLOAD_CHARS = Math.ceil(MAX_SOUND_BYTES / 3) * 4;
export const MAX_UPLOAD_PARTS = 256;

export type UploadStart = { upload: string; server: string; name: string; emoji?: string | null; parts: number };
export type UploadPart = { upload: string; index: number; data: string };

type Pending = { start: UploadStart; parts: (string | undefined)[]; received: number; chars: number };

export class UploadAssembler {
  #pending: Pending | null = null;

  start(start: UploadStart): boolean {
    this.#pending = null;
    if (!start.upload || !start.server || !Number.isInteger(start.parts) || start.parts < 1 || start.parts > MAX_UPLOAD_PARTS) {
      return false;
    }
    this.#pending = { start, parts: new Array(start.parts), received: 0, chars: 0 };
    return true;
  }

  add(part: UploadPart): NewSound | null {
    const pending = this.#pending;
    if (!pending || part.upload !== pending.start.upload || typeof part.data !== "string") return null;
    if (!Number.isInteger(part.index) || part.index < 0 || part.index >= pending.parts.length || pending.parts[part.index] !== undefined) {
      return null;
    }
    pending.chars += part.data.length;
    if (pending.chars > MAX_UPLOAD_CHARS) {
      this.#pending = null;
      throw new Error("That clip is over 1 MB, pick a shorter part");
    }
    pending.parts[part.index] = part.data;
    pending.received += 1;
    if (pending.received < pending.parts.length) return null;
    this.#pending = null;
    const { server, name, emoji } = pending.start;
    return { server, name, emoji: emoji ?? null, data: pending.parts.join("") };
  }
}
