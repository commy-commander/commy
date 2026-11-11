import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = read("styles", "phone-drive.css");
const glue = read("scripts", "phone-drive.js");
const html = read("index.html");
const SLIM = 'html[data-look="studio"][data-fit="slim"]';

const rule = selector => {
  const at = css.indexOf(`${SLIM} ${selector} {`);
  assert.notEqual(at, -1, selector);
  return css.slice(at, css.indexOf("}", at));
};

test("every phone drive rule is for the phone app in the new look, with tokens and no comments", () => {
  const selectors = css.split("}").map(block => block.split("{")[0]).flatMap(head => head.split(",")).map(s => s.trim()).filter(Boolean);
  assert.ok(selectors.length > 15);
  for (const selector of selectors) assert.ok(selector.startsWith(SLIM), selector);
  assert.equal(/#[0-9a-fA-F]{3,8}\b/.test(css.replace(/#drive-[a-z-]+|#chat-hash|#channel-name/g, "")), false);
  assert.equal(css.includes("/*"), false);
  assert.equal(/\/\/|\/\*/.test(glue), false);
});

test("the phone drive files sit with the other phone files", () => {
  assert.match(html, /id="studio-phone" href="styles\/phone-touch\.css" media="not all" \/>\n(\s*<link rel="stylesheet" id="studio-phone-[a-z-]+" href="styles\/phone-[a-z-]+\.css" media="not all" \/>\n)*\s*<link rel="stylesheet" id="studio-phone-drive" href="styles\/phone-drive\.css" media="not all" \/>/);
  assert.match(html, /<script src="scripts\/studio-phone\.js"><\/script>\n(\s*<script src="scripts\/phone-[a-z-]+\.js"><\/script>\n)*\s*<script src="scripts\/phone-drive\.js"><\/script>/);
});

test("drive on the phone app is a slab with a Beta capsule in the head and no crumbs", () => {
  assert.match(rule("body.mobile-app #drive-view"), /margin: 6px 10px calc\(-1 \* var\(--radius-xl\)\);\n  padding: 14px 12px 0;\n  border-radius: var\(--radius-xl\);/);
  assert.match(rule("body.mobile-app #drive-view .drive-crumbs"), /display: none;/);
  assert.match(rule("body.mobile-app #drive-view .drive-toolbar > .beta-tag"), /height: 22px;.*padding: 0 9px;.*border-radius: 999px; background: var\(--bg-active\);.*font: 700 12px\/22px var\(--font\);.*color: var\(--text-muted\);/);
});

test("the drive pane is the quiet well with the bold line and the app's line under it", () => {
  assert.match(rule("body.mobile-app #drive-view .drive-empty"), /display: grid; justify-items: center; gap: 8px; padding: 120px 18px 26px; text-align: center;/);
  assert.match(rule("body.mobile-app #drive-view .drive-empty > .pt-drive-well"), /width: 64px; height: 64px; margin-bottom: 6px; border-radius: 50%; background: var\(--input-bg\); box-shadow: var\(--slot\); color: var\(--text-faint\);/);
  assert.match(rule("body.mobile-app #drive-view .pt-drive-well > .i"), /width: 28px; height: 28px;/);
  assert.match(rule("body.mobile-app #drive-view .drive-empty > b"), /font: 700 17px\/1\.2 var\(--font-display\); color: var\(--header-primary\);/);
  assert.match(rule("body.mobile-app #drive-view .drive-empty > span"), /max-width: 340px;.*font: 400 14\.5px\/1\.45 var\(--font\); color: var\(--text-muted\);/);
  assert.match(glue, /const PHONE_DRIVE_LEAD = "Open this channel in the desktop app to see its files\.";/);
  assert.match(glue, /if \(!studioSlim\(\) \|\| driveWorksHere\(\)\) return;/);
  assert.match(glue, /studioIconMarkup\("folder"\)/);
});
