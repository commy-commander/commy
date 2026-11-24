import test from "node:test";
import assert from "node:assert/strict";
import { chromium, devices } from "playwright";
import { serveUi } from "./serve.mjs";

async function openEmptyHome(browser, url) {
  const context = await browser.newContext({ ...devices["iPhone 14"] });
  const page = await context.newPage();
  await page.goto(`${url}/index.html?mobile&ios&noservers`);
  await page.waitForFunction(() => typeof state !== "undefined" && state.user && Array.isArray(state.servers) && document.getElementById("startup").classList.contains("hidden"));
  await page.evaluate(() => hideModals());
  await page.locator("#horizon [data-tab=home]").tap();
  await page.waitForFunction(() => document.getElementById("app").dataset.tab === "home");
  return { context, page };
}

async function tapAndSee(browser, url, key, shown) {
  const { context, page } = await openEmptyHome(browser, url);
  try {
    assert.equal(await page.evaluate(() => state.servers.length), 0);
    await page.locator(key).tap();
    await page.waitForFunction(id => !document.getElementById(id).classList.contains("hidden"), shown, { timeout: 3000 });
  } finally {
    await context.close();
  }
}

test("Add on an empty phone Home opens the add a server window", async () => {
  const ui = await serveUi();
  const browser = await chromium.launch();
  try {
    await tapAndSee(browser, ui.url, "#add-server-btn", "modal-add-server");
  } finally {
    await browser.close();
    await ui.close();
  }
});

test("Discover on an empty phone Home opens Discover", async () => {
  const ui = await serveUi();
  const browser = await chromium.launch();
  try {
    await tapAndSee(browser, ui.url, "#discover-btn", "discover");
  } finally {
    await browser.close();
    await ui.close();
  }
});
