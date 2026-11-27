import test from "node:test";
import assert from "node:assert/strict";
import { chromium } from "playwright";
import { serveUi } from "./serve.mjs";

const SLACK = 0.5;
const DUAL_SCREEN = { width: 1108, height: 720, hinge: { orientation: "vertical", offset: 540, maskLength: 28 } };
const HALF_OPEN = { width: 1100, height: 840, hinge: { orientation: "horizontal", offset: 420, maskLength: 0 } };

async function openFolded(browser, url, device) {
  const context = await browser.newContext({ viewport: { width: device.width, height: device.height }, hasTouch: true, isMobile: true });
  const page = await context.newPage();
  const cdp = await context.newCDPSession(page);
  await cdp.send("Emulation.setDeviceMetricsOverride", { width: device.width, height: device.height, deviceScaleFactor: 1, mobile: true, displayFeature: device.hinge });
  await page.goto(`${url}/index.html?mobile&ios`);
  await page.waitForFunction(() => typeof state !== "undefined" && state.user && Array.isArray(state.servers) && state.servers.length > 0 && document.getElementById("startup").classList.contains("hidden"));
  await page.evaluate(() => hideModals());
  return page;
}

async function joinTheVoiceRoom(page) {
  await page.evaluate(async () => {
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
}

async function boxes(page, selectors) {
  return page.evaluate(list => Object.fromEntries(list.map(selector => {
    const rect = document.querySelector(selector).getBoundingClientRect();
    return [selector, { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom }];
  })), selectors);
}

async function withBrowser(run) {
  const ui = await serveUi();
  const browser = await chromium.launch();
  try {
    await run(browser, ui.url);
  } finally {
    await browser.close();
    await ui.close();
  }
}

test("on a dual screen the channels fill the left screen and the chat the right, clear of the hinge", () => withBrowser(async (browser, url) => {
  const page = await openFolded(browser, url, DUAL_SCREEN);
  assert.deepEqual(await page.evaluate(() => ({ fold: document.documentElement.dataset.fold, panes: document.documentElement.dataset.panes })), { fold: "book", panes: "2" });
  const found = await boxes(page, [".sidebar", ".main"]);
  const hingeLeft = DUAL_SCREEN.hinge.offset;
  const hingeRight = hingeLeft + DUAL_SCREEN.hinge.maskLength;
  assert.ok(found[".sidebar"].right <= hingeLeft + SLACK, `the channels end at ${found[".sidebar"].right}`);
  assert.ok(found[".sidebar"].right >= hingeLeft - 24, `the channels stop short at ${found[".sidebar"].right}`);
  assert.ok(found[".main"].left >= hingeRight - SLACK, `the chat starts at ${found[".main"].left}`);
}));

test("on a dual screen in a voice room the room fills the left screen and the chat the right", () => withBrowser(async (browser, url) => {
  const page = await openFolded(browser, url, DUAL_SCREEN);
  await joinTheVoiceRoom(page);
  const found = await boxes(page, ["#voice-room", "#voice-panel", ".main"]);
  const hingeLeft = DUAL_SCREEN.hinge.offset;
  const hingeRight = hingeLeft + DUAL_SCREEN.hinge.maskLength;
  assert.ok(found["#voice-room"].right <= hingeLeft + SLACK, `the room ends at ${found["#voice-room"].right}`);
  assert.ok(found["#voice-panel"].right <= hingeLeft + SLACK, `the keys end at ${found["#voice-panel"].right}`);
  assert.ok(found[".main"].left >= hingeRight - SLACK, `the chat starts at ${found[".main"].left}`);
}));

test("half open on a table, the room sits above the fold and the four keys and the chat below it", () => withBrowser(async (browser, url) => {
  const page = await openFolded(browser, url, HALF_OPEN);
  assert.equal(await page.evaluate(() => document.documentElement.dataset.fold), "tabletop");
  await joinTheVoiceRoom(page);
  const fold = HALF_OPEN.hinge.offset;
  const found = await boxes(page, ["#voice-room", "#voice-panel", ".main"]);
  assert.ok(found["#voice-room"].bottom <= fold + SLACK, `the room ends at ${found["#voice-room"].bottom}`);
  assert.ok(found["#voice-panel"].top >= fold - SLACK, `the keys start at ${found["#voice-panel"].top}`);
  assert.ok(found[".main"].top >= fold - SLACK, `the chat starts at ${found[".main"].top}`);
  assert.ok(found["#voice-panel"].right <= found[".main"].left + SLACK, "the keys run into the chat");
  const words = await page.evaluate(() => ["vp-more", "deafen-btn", "mute-btn", "vp-disconnect"].map(id => {
    const key = document.getElementById(id);
    return key.getClientRects().length > 0 ? (key.closest(".split-btn") || key).dataset.word : "";
  }));
  assert.deepEqual(words, ["More", "Deafen", "Mic", "Leave"]);
}));
