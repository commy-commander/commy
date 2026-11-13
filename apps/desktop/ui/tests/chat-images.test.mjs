import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { IMAGE_COPY_MIME, IMAGE_NAME_HEADER, IMAGE_ACTIONS, clipboardTakesImages, imageActions, imageNameFromUrl, imageSaveName } = require("../scripts/chat-images.js");

const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const glue = read("scripts", "chat-images.js");
const app = read("scripts", "app.js");
const minors = read("scripts", "studio-minors.js");
const phone = read("scripts", "studio-phone.js");
const html = read("index.html");

test("copy is offered only where the clipboard takes pictures", () => {
  const write = () => {};
  assert.equal(clipboardTakesImages({ clipboard: { write } }, function Item() {}), true);
  assert.equal(clipboardTakesImages({ clipboard: { writeText: write } }, function Item() {}), false);
  assert.equal(clipboardTakesImages({ clipboard: { write } }, null), false);
  assert.equal(clipboardTakesImages({}, function Item() {}), false);
  assert.equal(clipboardTakesImages(undefined, function Item() {}), false);
});

test("desktop right-click offers copy, save and the link of a linked picture", () => {
  assert.deepEqual(imageActions({ url: "https://media.giphy.com/a.gif" }, "menu", true), ["copy", "save", "link"]);
  assert.deepEqual(imageActions({ att: { filename: "cat.png" } }, "menu", true), ["copy", "save"]);
  assert.deepEqual(imageActions({}, "menu", false), ["save"]);
  assert.deepEqual(["copy", "save", "link"].map(action => IMAGE_ACTIONS[action].menu), ["Copy Image", "Save Image", "Copy Image Link"]);
});

test("holding a picture on a phone offers copy and save", () => {
  assert.deepEqual(imageActions({ url: "https://media.giphy.com/a.gif" }, "sheet", true), ["copy", "save"]);
  assert.deepEqual(imageActions({ att: { filename: "cat.png" } }, "sheet", false), ["save"]);
  assert.deepEqual(["copy", "save"].map(action => IMAGE_ACTIONS[action].sheet), ["Copy Image", "Save Image"]);
});

test("a saved picture keeps its name and gets the ending its bytes need", () => {
  assert.equal(imageNameFromUrl("https://media2.giphy.com/media/v1.abc/giphy.gif?cid=1"), "giphy.gif");
  assert.equal(imageNameFromUrl("https://pics.example.org/my%20cat.png"), "my cat.png");
  assert.equal(imageNameFromUrl("https://pics.example.org/"), "");
  assert.equal(imageNameFromUrl("not a url"), "");
  assert.equal(imageSaveName("cat.png", null, "image/png"), "cat.png");
  assert.equal(imageSaveName("photo", null, "image/jpeg"), "photo.jpg");
  assert.equal(imageSaveName("shot.JPEG", null, "image/jpeg"), "shot.JPEG");
  assert.equal(imageSaveName(null, "https://media.tenor.com/x/happy.gif", "image/gif"), "happy.gif");
  assert.equal(imageSaveName(null, null, "image/webp; charset=binary"), "image.webp");
  assert.equal(imageSaveName(null, null, ""), "image");
  assert.equal(imageSaveName("a/b:c?.png", null, "image/png"), "a_b_c_.png");
});

test("copying writes a png made from the picture, and falls back to its link", () => {
  assert.equal(IMAGE_COPY_MIME, "image/png");
  assert.match(glue, /item = new ClipboardItem\(\{ \[IMAGE_COPY_MIME\]: png \}\);/);
  assert.match(glue, /return navigator\.clipboard\.write\(\[item\]\)\.then\(/);
  assert.match(glue, /if \(source && source\.url\) copyText\(source\.url, "Copied the image link instead"\);/);
  assert.match(glue, /if \(source\.att && source\.att\.filename\) return saveAttachment\(source\.att\);/);
  assert.equal(IMAGE_NAME_HEADER, "x-commy-name");
  assert.match(glue, /invoke\("save_image", bytes, \{ headers: \{ \[IMAGE_NAME_HEADER\]: encodeURIComponent\(name\) \} \}\)/);
});

test("chat pictures are marked so menus can find them", () => {
  assert.match(app, /const img = markChatImage\(el\("img", "chat-image"\), \{ att, name: att\.filename \}\);/);
  assert.match(app, /const img = markChatImage\(el\("img", "chat-image"\), \{ url \}\);/);
  assert.match(app, /const img = markChatImage\(el\("img"\), \{\}\);/);
  assert.match(html, /<script src="scripts\/embeds\.js"><\/script>\n\s*<script src="scripts\/chat-images\.js"><\/script>/);
});

test("right-clicking a picture on desktop puts the picture items above the message items", () => {
  assert.match(minors, /const image = chatImageAt\(e\.target\);/);
  assert.match(minors, /openMessageMenu\(message, minorsPointAnchor\(e\.clientX, e\.clientY\), image\);/);
  assert.match(app, /if \(image && chatImageMenuItems\(item, image\)\) sep\(\);/);
});

test("holding a picture on a phone lists copy and save first, and copy runs inside the tap", () => {
  assert.match(phone, /if \(!openHoldSheet\(msg, chatImageAt\(e\.target\)\)\) return;/);
  assert.match(phone, /if \(image\) for \(const \{ title, run \} of chatImageSheetActions\(image\)\) rows\.appendChild\(holdImageRow\(title, run\)\);/);
  assert.match(phone, /row\.onclick = \(\) => \{\n    run\(\);\n    closeSheets\(\);\n  \};/);
});

test("the picture helpers carry no comments", () => {
  assert.equal(/\/\/|\/\*/.test(glue.replace(/https?:\/\//g, "")), false);
});
