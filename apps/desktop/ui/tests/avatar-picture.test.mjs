import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { AVATAR_MAX_BYTES, pictureKind, avatarPicturePlan, avatarGifsOffered } = require("../scripts/avatar-picture.js");

const bytes = (text, size = text.length) => {
  const out = new Uint8Array(size);
  [...text].forEach((c, i) => { out[i] = c.charCodeAt(0); });
  return out;
};
const png = size => { const b = bytes("_PNG", size); b[0] = 0x89; return b; };
const jpeg = size => { const b = new Uint8Array(size); b.set([0xff, 0xd8, 0xff]); return b; };

test("pictures are told apart by their first bytes", () => {
  assert.equal(pictureKind(bytes("GIF89a....")), "gif");
  assert.equal(pictureKind(bytes("GIF87a....")), "gif");
  assert.equal(pictureKind(png(8)), "png");
  assert.equal(pictureKind(jpeg(8)), "jpeg");
  assert.equal(pictureKind(bytes("RIFF\0\0\0\0WEBPVP8 ")), "webp");
  assert.equal(pictureKind(bytes("<svg>")), null);
  assert.equal(pictureKind(null), null);
});

test("an animated GIF avatar is kept as it is so it keeps moving, up to the size limit", () => {
  assert.equal(avatarPicturePlan(bytes("GIF89a", 1000)), "keep");
  assert.equal(avatarPicturePlan(bytes("GIF89a", AVATAR_MAX_BYTES)), "keep");
  assert.equal(avatarPicturePlan(bytes("GIF89a", AVATAR_MAX_BYTES + 1)), "too-large");
});

test("uploaded photos are redrawn, while small still emojis keep their own pixels", () => {
  assert.equal(avatarPicturePlan(jpeg(1000)), "redraw");
  assert.equal(avatarPicturePlan(png(1000)), "redraw");
  assert.equal(avatarPicturePlan(png(1000), { keepStill: true }), "keep");
  assert.equal(avatarPicturePlan(jpeg(1000), { keepStill: true }), "keep");
  assert.equal(avatarPicturePlan(png(AVATAR_MAX_BYTES + 1), { keepStill: true }), "redraw");
  assert.equal(avatarPicturePlan(bytes("RIFF\0\0\0\0WEBPVP8 "), { keepStill: true }), "redraw");
});

test("GIF avatars are offered only where picked GIFs can be fetched", () => {
  assert.equal(avatarGifsOffered({ provider: "klipy", key: "" }, true), true);
  assert.equal(avatarGifsOffered({ provider: "klipy", key: "" }, false), false);
  assert.equal(avatarGifsOffered({ provider: "klipy", key: "k" }, false), true);
  assert.equal(avatarGifsOffered({ provider: "giphy", key: "k" }, true), false);
});
