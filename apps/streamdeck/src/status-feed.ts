import type { CommyLink } from "./commy-link.ts";
import type { DeckStatus } from "./protocol.ts";

export const STATUS_EVERY_MS = 1000;
export const RECONNECT_EVERY_MS = 5000;

type Listener = (status: DeckStatus | null) => void;

export class StatusFeed {
  #link: CommyLink;
  #everyMs: number;
  #reconnectMs: number;
  #listeners = new Set<Listener>();
  #timer: NodeJS.Timeout | null = null;
  #last: DeckStatus | null = null;
  #wasReady = false;
  #triedOnce = false;
  #lastReconnect = 0;
  #busy = false;

  constructor(link: CommyLink, everyMs = STATUS_EVERY_MS, reconnectMs = RECONNECT_EVERY_MS) {
    this.#link = link;
    this.#everyMs = everyMs;
    this.#reconnectMs = reconnectMs;
    link.onState(state => {
      if (state === "ready") {
        this.#wasReady = true;
        void this.refresh();
      } else {
        this.publish(null);
      }
    });
  }

  get last(): DeckStatus | null {
    return this.#last;
  }

  watch(listener: Listener): () => void {
    this.#listeners.add(listener);
    if (!this.#timer) {
      this.#timer = setInterval(() => void this.refresh(), this.#everyMs);
      void this.refresh();
    }
    return () => {
      this.#listeners.delete(listener);
      if (this.#listeners.size === 0 && this.#timer) {
        clearInterval(this.#timer);
        this.#timer = null;
      }
    };
  }

  publish(status: DeckStatus | null): void {
    this.#last = status;
    for (const listener of this.#listeners) listener(status);
  }

  async refresh(): Promise<void> {
    if (this.#busy) return;
    this.#busy = true;
    try {
      if (this.#link.state !== "ready") {
        const retry = this.#wasReady && this.#link.state === "offline" && Date.now() - this.#lastReconnect >= this.#reconnectMs;
        if (this.#triedOnce && !retry) return;
        this.#triedOnce = true;
        this.#lastReconnect = Date.now();
        if ((await this.#link.connect()) !== "ready") return;
      }
      this.publish(await this.#link.status());
    } catch {
      this.publish(null);
    } finally {
      this.#busy = false;
    }
  }
}
