import streamDeck from "@elgato/streamdeck";

import type { TokenStore } from "./commy-link.ts";

type GlobalSettings = { tokens?: Record<string, string> };

async function tokens(): Promise<Record<string, string>> {
  const settings = await streamDeck.settings.getGlobalSettings<GlobalSettings>();
  return { ...(settings.tokens ?? {}) };
}

async function writeTokens(next: Record<string, string>): Promise<void> {
  await streamDeck.settings.setGlobalSettings<GlobalSettings>({ tokens: next });
}

export const globalTokenStore: TokenStore = {
  async load(port) {
    return (await tokens())[String(port)];
  },
  async save(port, token) {
    await writeTokens({ ...(await tokens()), [String(port)]: token });
  },
  async forget(port) {
    const next = await tokens();
    delete next[String(port)];
    await writeTokens(next);
  },
};
