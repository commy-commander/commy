import test from "node:test";
import assert from "node:assert/strict";
import { chromium } from "playwright";
import { serveUi } from "./serve.mjs";

const RAIL_TABLETS = [
  { name: "iPad upright", width: 834, height: 1194 },
  { name: "iPad on its side", width: 1194, height: 834 },
  { name: "Android tablet upright", width: 800, height: 1280 },
  { name: "Android tablet on its side", width: 1280, height: 800 },
];

async function openTablet(browser, url, size) {
  const context = await browser.newContext({ viewport: { width: size.width, height: size.height }, hasTouch: true, isMobile: true });
  const page = await context.newPage();
  await page.goto(`${url}/index.html?mobile&ios`);
  await page.waitForFunction(() => typeof state !== "undefined" && state.user && Array.isArray(state.servers) && state.servers.length > 0 && document.getElementById("startup").classList.contains("hidden"));
  await page.evaluate(() => hideModals());
  return page;
}

async function measureRail(page) {
  return page.evaluate(() => {
    const box = node => {
      const rect = node.getBoundingClientRect();
      return { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom, width: rect.width, height: rect.height };
    };
    const shown = node => node.getClientRects().length > 0;
    const keys = ["home-btn", "add-server-btn", "discover-btn", "user-chip", "settings-btn"].map(id => ({ id, ...box(document.getElementById(id)) }));
    const servers = [...document.querySelectorAll("#server-list .server-icon")].filter(shown).map(node => ({ id: "server", ...box(node) }));
    return { rail: box(document.getElementById("rail")), sidebar: box(document.querySelector(".sidebar")), main: box(document.querySelector(".main")), keys: [...keys, ...servers], view: { width: innerWidth, height: innerHeight } };
  });
}

for (const size of RAIL_TABLETS) {
  test(`${size.name}: the servers, DMs and you sit in a rail down the left edge, clear of the channels`, async () => {
    const ui = await serveUi();
    const browser = await chromium.launch();
    try {
      const page = await openTablet(browser, ui.url, size);
      const { rail, sidebar, main, keys, view } = await measureRail(page);
      assert.ok(rail.height > rail.width * 4, `the rail is ${rail.width}x${rail.height}`);
      assert.ok(rail.right <= sidebar.left, "the rail runs into the channels");
      assert.ok(sidebar.top < 40 && main.top < 40, "the channels and chat start at the top");
      for (const key of keys) {
        assert.ok(key.width >= 44 && key.height >= 44, `${key.id} is ${key.width}x${key.height}`);
        assert.ok(key.left >= rail.left && key.right <= rail.right, `${key.id} sticks out of the rail`);
        assert.ok(key.top >= 0 && key.bottom <= view.height, `${key.id} is off screen`);
      }
    } finally {
      await browser.close();
      await ui.close();
    }
  });
}

test("an open dual screen keeps the servers along the top, where a rail would squeeze the chat", async () => {
  const ui = await serveUi();
  const browser = await chromium.launch();
  try {
    const page = await openTablet(browser, ui.url, { width: 669, height: 951 });
    const { rail, sidebar } = await measureRail(page);
    assert.ok(rail.width > rail.height, `the rail is ${rail.width}x${rail.height}`);
    assert.ok(rail.bottom <= sidebar.top);
  } finally {
    await browser.close();
    await ui.close();
  }
});
