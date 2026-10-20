import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { linksIn, previewableLink, previewLayout, previewImageBox, previewSite, imageGridClass, readLinkPreviewPrefs, PREVIEW_CARD_WIDTH, PREVIEW_SIDE_IMAGE, PREVIEW_IMAGE_MAX_HEIGHT } = require("../scripts/link-preview.js");

test("links lose the punctuation that ends a sentence but keep balanced brackets", () => {
  assert.deepEqual(linksIn("see https://example.org/a. and (https://example.org/b), or https://en.wikipedia.org/wiki/Rust_(language)!"), [
    "https://example.org/a",
    "https://example.org/b",
    "https://en.wikipedia.org/wiki/Rust_(language)",
  ]);
  assert.deepEqual(linksIn(""), []);
  assert.deepEqual(linksIn(undefined), []);
});

test("the first link that is not skipped gets the preview", () => {
  const skip = url => url.endsWith(".png");
  assert.equal(previewableLink("cat https://pics.example.org/cat.png then https://example.org/story", skip), "https://example.org/story");
  assert.equal(previewableLink("only https://pics.example.org/cat.png", skip), null);
  assert.equal(previewableLink("no links here"), null);
  assert.equal(previewableLink("broken https://"), null);
});

test("wide pictures sit under the text, small or square ones at the side", () => {
  assert.equal(previewLayout({ image: { width: 1200, height: 630 } }), "wide");
  assert.equal(previewLayout({ image: { width: 400, height: 400 } }), "side");
  assert.equal(previewLayout({ image: { width: 200, height: 100 } }), "side");
  assert.equal(previewLayout({ image: null }), "text");
  assert.equal(previewLayout({ image: { width: 0, height: 10 } }), "text");
  assert.equal(previewLayout({}), "text");
});

test("the picture's space is fixed from its size, so the card does not move as it loads", () => {
  assert.deepEqual(previewImageBox({ width: 80, height: 300 }, "side"), { width: PREVIEW_SIDE_IMAGE, height: PREVIEW_SIDE_IMAGE });
  const wide = previewImageBox({ width: 640, height: 336 }, "wide");
  assert.equal(wide.width, PREVIEW_CARD_WIDTH - 30);
  assert.equal(wide.height, Math.round((wide.width * 336) / 640));
  assert.equal(previewImageBox({ width: 640, height: 600 }, "wide").height, PREVIEW_IMAGE_MAX_HEIGHT);
  assert.deepEqual(previewImageBox({ width: 320, height: 160 }, "wide"), { width: 320, height: 160 });
});

test("the card names the host the link really goes to, whatever name the sender's copy gives", () => {
  assert.equal(previewSite({ site: "Your Bank", url: "https://www.example.org" }), "example.org");
  assert.equal(previewSite({ site: "", url: "https://news.example.org/a" }), "news.example.org");
  assert.equal(previewSite({ site: "Example", url: "nonsense" }), "Example");
  assert.equal(previewSite({ site: "", url: "nonsense" }), "");
});

test("two to four images share a grid", () => {
  assert.equal(imageGridClass(1), "");
  assert.equal(imageGridClass(2), "image-grid grid-2");
  assert.equal(imageGridClass(4), "image-grid grid-4");
  assert.equal(imageGridClass(9), "image-grid grid-4");
});

test("preview settings default to on and survive bad saved values", () => {
  assert.deepEqual(readLinkPreviewPrefs(null), { send: true, show: true });
  assert.deepEqual(readLinkPreviewPrefs("not json"), { send: true, show: true });
  assert.deepEqual(readLinkPreviewPrefs('{"send":false}'), { send: false, show: true });
  assert.deepEqual(readLinkPreviewPrefs('{"send":"no","show":false}'), { send: true, show: false });
});
