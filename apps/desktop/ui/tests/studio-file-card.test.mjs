import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const parts = read("../styles/studio-parts.css");
const phone = read("../styles/phone-touch.css");
const app = read("../scripts/app.js");

const LOOK = 'html[data-look="studio"]';
const rule = selector => {
  const start = parts.indexOf(`${LOOK} ${selector} {`);
  assert.ok(start >= 0, selector);
  return parts.slice(start, parts.indexOf("}", start));
};

test("a file that is not a picture is the guide's file card in the new look", () => {
  const card = rule(".attachment.filecard");
  for (const line of ["width: 340px;", "max-width: 100%;", "height: 64px;", "padding: 0 12px;", "border-radius: 22px;", "box-shadow: var(--pillow-card);"]) assert.ok(card.includes(line), line);
  const plate = rule(".filecard .plate");
  for (const line of ["width: 42px;", "height: 42px;", "border-radius: 13px;", "background: var(--input-bg);", "color: var(--accent-lit);", "box-shadow: var(--slot);"]) assert.ok(plate.includes(line), line);
  assert.ok(rule(".filecard .file-name").includes("font: 600 14.5px/1 var(--font);"));
  assert.ok(rule(".filecard .file-size").includes("font: 400 13px/1 var(--font);"));
  assert.ok(rule(".filecard .key.file-save").includes("width: 38px; height: 38px;"));
  assert.ok(phone.includes(`${LOOK}[data-fit="slim"] .filecard .key.file-save::after { content: ""; position: absolute; inset: -3px; border-radius: 50%; }`));
});

test("the file card keeps the classic card and its download words outside the new look", () => {
  const start = app.indexOf("function renderAttachment(att) {");
  const body = app.slice(start, app.indexOf("\n}\n", start));
  assert.match(body, /const box = el\("div", studio \? "attachment filecard" : "attachment"\);/);
  assert.match(body, /studio \? el\("button", "key sm sq file-save"\) : el\("button", "btn-secondary btn-small", "Download"\)/);
  assert.match(body, /btn\.setAttribute\("aria-label", `Save \$\{name\}`\);/);
  assert.match(body, /const words = studio \? size : btn;/);
  assert.match(body, /words\.textContent = isBigFile\(att\.size_bytes\) \? "Choose where" : "Fetching";/);
  assert.match(body, /const failed = studio && !saved;\n    words\.textContent = failed \? "Couldn't save" : rest;/);
  assert.match(body, /btn\.title = failed \? "Retry" : `Save \$\{name\}`;/);
  assert.ok(parts.includes(`${LOOK} .filecard .file-size.file-failed { color: var(--danger); }`));
  const save = app.slice(app.indexOf("async function saveAttachment(att) {"));
  assert.match(save.slice(0, save.indexOf("\n}\n")), /return true;\n  \} catch \(e\) \{\n    toast\(String\(e\)\);\n    return false;/);
});
