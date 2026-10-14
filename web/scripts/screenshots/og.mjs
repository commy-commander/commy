import { chromium } from "playwright";
const b = await chromium.launch(); const p = await b.newPage({ viewport: { width: 1200, height: 630 } });
await p.goto("file:///work/og.html"); await p.waitForTimeout(500);
await p.screenshot({ path: "/work/out/og.png" }); await b.close(); console.log("og done");
