// Renders public/og.png from og.html. Run from this folder: node og.mjs
import { chromium } from "playwright";
import path from "node:path";

const here = path.dirname(new URL(import.meta.url).pathname);
const browser = await chromium.launch({ args: process.env.NO_SANDBOX ? ["--no-sandbox"] : [] });
const page = await browser.newPage({ viewport: { width: 1200, height: 630 } });
await page.goto(`file://${path.join(here, "og.html")}`);
await page.waitForTimeout(800);
await page.screenshot({ path: process.env.OUT || path.join(here, "..", "..", "public", "og.png") });
await browser.close();
console.log("og done");
