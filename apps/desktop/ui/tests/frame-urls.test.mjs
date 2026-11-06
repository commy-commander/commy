import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { createFrameUrls, FRAME_URLS_KEPT } = require("../scripts/frame-urls.js");

test("each stream keeps only its last few frames alive, so watching does not fill memory", () => {
  let n = 0;
  const revoked = [];
  const made = [];
  const urls = createFrameUrls({ make: bytes => { made.push(bytes); return `blob:${++n}`; }, revoke: url => revoked.push(url) });
  const jpeg = Buffer.from([0xff, 0xd8, 0xff, 0xd9]).toString("base64");
  for (let i = 0; i < 100; i++) urls.from("stage", jpeg);
  assert.equal(urls.held("stage"), FRAME_URLS_KEPT);
  assert.equal(revoked.length, 100 - FRAME_URLS_KEPT);
  assert.deepEqual([...made[0]], [0xff, 0xd8, 0xff, 0xd9], "the picture bytes come through unchanged");
  urls.from("cam:a", jpeg);
  urls.drop("stage");
  assert.equal(urls.held("stage"), 0);
  assert.equal(urls.held("cam:a"), 1, "another stream keeps its frame");
  assert.equal(revoked.length, 100);
});
