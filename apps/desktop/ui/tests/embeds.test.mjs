import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { MAX_IMAGE_EMBEDS, isMediaHost, imageHost, imageLinks, embedMode, imageGateText } = require("../scripts/embeds.js");

const KLIPY = "https://static.klipy.com/ii/4e7b/fd/3b/5wGhb3j5.gif";
const GIPHY = "https://media2.giphy.com/media/v1.Y2lkPTc5MGI3NjExYWJj/giphy.gif?cid=790b7611abc&rid=giphy.gif&ct=g";
const OTHER = "https://pics.example.org/cat.png";

test("only the hosts the account server proxies count as media hosts", () => {
  for (const host of ["static.klipy.com", "images.igdb.com", "media.giphy.com", "media0.giphy.com", "media4.giphy.com", "media9.giphy.com", "i.giphy.com", "I.GIPHY.COM"]) {
    assert.ok(isMediaHost(host), host);
  }
  for (const host of ["giphy.com", "api.giphy.com", "media10.giphy.com", "media0.giphy.com.evil.example", "evilmedia0.giphy.com", "static.klipy.com.evil.example", "klipy.com", "example.org", "", null]) {
    assert.equal(isMediaHost(host), false, String(host));
  }
});

test("the host of an image link is read from the URL, https only", () => {
  assert.equal(imageHost(OTHER), "pics.example.org");
  assert.equal(imageHost("https://static.klipy.com@evil.example/a.gif"), "evil.example");
  assert.equal(imageHost("http://pics.example.org/cat.png"), null);
  assert.equal(imageHost("not a url"), null);
  assert.equal(imageHost(""), null);
  assert.equal(imageHost(undefined), null);
});

test("image links are https links with an image extension, at most four", () => {
  assert.deepEqual(imageLinks(`look ${OTHER} and ${GIPHY}`), [OTHER, GIPHY]);
  assert.deepEqual(imageLinks("https://a.example/x.webp?size=2 https://b.example/y.jpeg"), ["https://a.example/x.webp?size=2", "https://b.example/y.jpeg"]);
  assert.deepEqual(imageLinks("http://a.example/x.gif"), [], "plain http is never embedded");
  assert.deepEqual(imageLinks("https://a.example/photo"), [], "a lone link without an extension is never embedded");
  assert.deepEqual(imageLinks("https://a.example/page.html"), []);
  assert.deepEqual(imageLinks("https://a.example/page?next=x.gif"), [], "the extension has to end the path");
  assert.deepEqual(imageLinks(""), []);
  assert.deepEqual(imageLinks(null), []);
  const many = Array.from({ length: 6 }, (_, i) => `https://a.example/${i}.png`);
  assert.equal(imageLinks(many.join(" ")).length, MAX_IMAGE_EMBEDS);
});

test("proxied hosts embed on their own, everything else waits for a click", () => {
  assert.equal(embedMode(KLIPY), "auto");
  assert.equal(embedMode(GIPHY), "auto");
  assert.equal(embedMode("https://images.igdb.com/igdb/image/upload/t_thumb/co1rs4.jpg"), "auto");
  assert.equal(embedMode(OTHER), "click");
  assert.equal(embedMode("https://static.klipy.com@evil.example/a.gif"), "click");
  assert.equal(embedMode(KLIPY, false), "click", "without the account server route nothing loads on its own");
  assert.equal(embedMode("http://static.klipy.com/a.gif"), "none");
  assert.equal(embedMode("nope"), "none");
});

test("the placeholder names the host in every line it shows", () => {
  const text = imageGateText("pics.example.org");
  assert.equal(text.title, "Image from pics.example.org");
  assert.equal(text.load, "Load");
  assert.equal(text.hint, "Loading fetches it from pics.example.org, so that site sees your address.");
  assert.match(text.failed, /pics\.example\.org/);
  assert.ok(text.loading && text.retry);
});
