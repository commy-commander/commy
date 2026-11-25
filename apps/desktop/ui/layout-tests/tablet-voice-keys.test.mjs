import test from "node:test";
import assert from "node:assert/strict";
import { chromium } from "playwright";
import { serveUi } from "./serve.mjs";

const TABLETS = [
  { name: "iPad upright", width: 834, height: 1194, ios: true },
  { name: "Android tablet upright", width: 800, height: 1280, ios: false },
  { name: "open dual screen", width: 669, height: 951, ios: false },
];

async function openTabletVoiceRoom(browser, url, tablet) {
  const context = await browser.newContext({ viewport: { width: tablet.width, height: tablet.height }, hasTouch: true, isMobile: true });
  const page = await context.newPage();
  await page.goto(`${url}/index.html?mobile${tablet.ios ? "&ios" : ""}`);
  await page.waitForFunction(() => typeof state !== "undefined" && state.user && Array.isArray(state.servers) && state.servers.length > 0 && document.getElementById("startup").classList.contains("hidden"));
  await page.evaluate(async () => {
    hideModals();
    const busy = Object.values(mock.servers).find(server => (server.channels || []).some(channel => (channel.voice_user_ids || []).length > 3));
    busy.needs_rules = false;
    await selectServer(busy.id);
    const voice = state.currentServer.channels.find(channel => channel.kind === "voice");
    await selectChannel(voice);
    await toggleVoice(voice);
    await refreshCurrentServer();
    await showCallGrid();
    placeVoiceRoom();
  });
  return { context, page };
}

async function shownKeys(page) {
  return page.evaluate(() => {
    const bar = document.getElementById("voice-panel");
    const shown = node => node.getClientRects().length > 0 && getComputedStyle(node).visibility !== "hidden";
    const keys = [...bar.querySelectorAll("button")].filter(shown).filter(node => !node.closest(".vp-live, .vp-watch, .vp-stage"));
    const box = bar.getBoundingClientRect();
    return keys
      .map(key => {
        const rect = key.getBoundingClientRect();
        const holder = key.closest(".split-btn") || key;
        return { id: key.id, word: holder.dataset.word || "", left: rect.left, inside: rect.left >= box.left && rect.right <= box.right && rect.top >= box.top && rect.bottom <= box.bottom, size: Math.min(rect.width, rect.height) };
      })
      .sort((a, b) => a.left - b.left);
  });
}

for (const tablet of TABLETS) {
  test(`${tablet.name}: the voice room bar is More, Deafen, Mic, Leave, each labelled and big enough to tap`, async () => {
    const ui = await serveUi();
    const browser = await chromium.launch();
    try {
      const { page } = await openTabletVoiceRoom(browser, ui.url, tablet);
      assert.equal(await page.evaluate(() => document.documentElement.dataset.panes), "2");
      const keys = await shownKeys(page);
      assert.deepEqual(keys.map(key => key.id), ["vp-more", "deafen-btn", "mute-btn", "vp-disconnect"]);
      assert.deepEqual(keys.map(key => key.word), ["More", "Deafen", "Mic", "Leave"]);
      for (const key of keys) {
        assert.ok(key.inside, `${key.id} sticks out of the bar`);
        assert.ok(key.size >= 44, `${key.id} is ${key.size}px`);
      }
    } finally {
      await browser.close();
      await ui.close();
    }
  });

  test(`${tablet.name}: More holds the camera and soundboard, and Share only where the screen can be shared`, async () => {
    const ui = await serveUi();
    const browser = await chromium.launch();
    try {
      const { page } = await openTabletVoiceRoom(browser, ui.url, tablet);
      await page.locator("#vp-more").tap();
      await page.waitForFunction(() => !document.getElementById("vp-more-menu").classList.contains("hidden"));
      const rows = await page.evaluate(() => [...document.querySelectorAll("#vp-more-menu .vp-more-row")].map(row => row.textContent));
      const share = tablet.ios ? [] : ["Share Screen"];
      assert.deepEqual(rows, ["Turn On Camera", ...share, "Soundboard"]);
    } finally {
      await browser.close();
      await ui.close();
    }
  });
}
