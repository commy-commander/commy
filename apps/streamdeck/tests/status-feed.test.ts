import assert from "node:assert/strict";
import test from "node:test";

import type { CommyLink, LinkState } from "../src/commy-link.ts";
import type { DeckStatus } from "../src/protocol.ts";
import { StatusFeed } from "../src/status-feed.ts";

class FakeLink {
  state: LinkState = "ready";
  asked = 0;
  connects = 0;
  #listeners = new Set<(state: LinkState) => void>();

  onState(listener: (state: LinkState) => void): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  setState(state: LinkState): void {
    this.state = state;
    for (const listener of this.#listeners) listener(state);
  }

  async connect(): Promise<LinkState> {
    this.connects += 1;
    return this.state;
  }

  async status(): Promise<DeckStatus> {
    this.asked += 1;
    return { muted: this.asked % 2 === 1 } as DeckStatus;
  }
}

const tick = () => new Promise(resolve => setTimeout(resolve, 0));

test("the feed asks only while someone watches", async () => {
  const link = new FakeLink();
  const feed = new StatusFeed(link as unknown as CommyLink, 10);
  assert.equal(link.asked, 0);
  const seen: (DeckStatus | null)[] = [];
  const stop = feed.watch(status => seen.push(status));
  await new Promise(resolve => setTimeout(resolve, 45));
  stop();
  const asked = link.asked;
  assert.ok(asked >= 2, `asked ${asked}`);
  await new Promise(resolve => setTimeout(resolve, 30));
  assert.equal(link.asked, asked);
  assert.equal(seen[0]?.muted, true);
});

test("a feed that was never ready does not keep asking Commy to pair", async () => {
  const link = new FakeLink();
  link.state = "declined";
  const feed = new StatusFeed(link as unknown as CommyLink, 10, 0);
  const stop = feed.watch(() => {});
  await new Promise(resolve => setTimeout(resolve, 40));
  stop();
  assert.equal(link.connects, 0);
  assert.equal(link.asked, 0);
});

test("after Commy restarts the feed reconnects and clears keys meanwhile", async () => {
  const link = new FakeLink();
  const feed = new StatusFeed(link as unknown as CommyLink, 1000, 0);
  const seen: (DeckStatus | null)[] = [];
  const stop = feed.watch(status => seen.push(status));
  link.setState("ready");
  await tick();
  link.setState("offline");
  assert.equal(seen.at(-1), null);
  await feed.refresh();
  assert.equal(link.connects, 1);
  stop();
});
