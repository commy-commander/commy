import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { AVATAR_CROP_MAX_ZOOM, avatarCropStart, avatarCropMove, avatarCropZoom, avatarCropSource } = require("../scripts/avatar-crop.js");

const near = (actual, expected) => assert.ok(Math.abs(actual - expected) < 1e-9, `${actual} is not ${expected}`);

test("a wide picture starts centred and fills the circle on its short side", () => {
  const crop = avatarCropStart(1000, 500, 280);
  near(crop.y, 0);
  near(crop.x, (280 - 560) / 2);
  const source = avatarCropSource(crop);
  near(source.sw, 500);
  near(source.sh, 500);
  near(source.sx, 250);
  near(source.sy, 0);
});

test("a tall picture starts centred on its long side", () => {
  const source = avatarCropSource(avatarCropStart(300, 900, 300));
  near(source.sw, 300);
  near(source.sh, 300);
  near(source.sx, 0);
  near(source.sy, 300);
});

test("dragging never uncovers the circle", () => {
  const start = avatarCropStart(1000, 500, 280);
  const left = avatarCropMove(start, 5000, 5000);
  near(left.x, 0);
  near(left.y, 0);
  const right = avatarCropMove(start, -5000, -5000);
  near(right.x, 280 - 560);
  near(right.y, 0);
  near(avatarCropSource(right).sx, 500);
});

test("zooming keeps the point under the pointer still", () => {
  const start = avatarCropStart(800, 800, 200);
  const at = { x: 50, y: 150 };
  const before = { x: (at.x - start.x) / 200 * 800, y: (at.y - start.y) / 200 * 800 };
  const zoomed = avatarCropZoom(start, 2, at);
  const scale = 200 / 800 * 2;
  near((at.x - zoomed.x) / scale, before.x);
  near((at.y - zoomed.y) / scale, before.y);
  near(avatarCropSource(zoomed).sw, 400);
});

test("zoom stays between the whole picture and four times closer", () => {
  const start = avatarCropStart(600, 400, 240);
  assert.equal(avatarCropZoom(start, 0.2).zoom, 1);
  assert.equal(avatarCropZoom(start, 99).zoom, AVATAR_CROP_MAX_ZOOM);
  assert.equal(avatarCropZoom(start, "nope").zoom, 1);
  const back = avatarCropZoom(avatarCropZoom(start, 3, { x: 0, y: 0 }), 1);
  near(back.y, 0);
  assert.ok(back.x <= 0 && back.x >= 240 - 360);
});

test("the cut stays inside the picture at any zoom and drag", () => {
  let crop = avatarCropStart(1200, 700, 280);
  for (const [zoom, dx, dy] of [[2.5, 90, -40], [4, -800, 300], [1.3, 60, 60], [3.7, -15, -999]]) {
    crop = avatarCropMove(avatarCropZoom(crop, zoom, { x: 100, y: 180 }), dx, dy);
    const { sx, sy, sw, sh } = avatarCropSource(crop);
    assert.ok(sx >= -1e-9 && sy >= -1e-9, `${sx} ${sy}`);
    assert.ok(sx + sw <= 1200 + 1e-9 && sy + sh <= 700 + 1e-9, `${sx + sw} ${sy + sh}`);
  }
});

test("a banner frame fills its width and keeps the banner's shape", () => {
  const crop = avatarCropStart(1200, 1600, 330, 132);
  const { sx, sy, sw, sh } = avatarCropSource(crop);
  near(sx, 0);
  near(sw, 1200);
  near(sw / sh, 330 / 132);
  near(sy, (1600 - sh) / 2);
});

test("a banner can be dragged up and down but never past the picture", () => {
  const start = avatarCropStart(1200, 1600, 330, 132);
  const top = avatarCropMove(start, 0, 9999);
  near(avatarCropSource(top).sy, 0);
  const bottom = avatarCropMove(start, 0, -9999);
  const cut = avatarCropSource(bottom);
  near(cut.sy + cut.sh, 1600);
  near(avatarCropMove(start, 500, 0).x, 0);
});

test("a picture shorter than the banner shape is matched on its height", () => {
  const { sx, sy, sw, sh } = avatarCropSource(avatarCropStart(2000, 400, 300, 100));
  near(sy, 0);
  near(sh, 400);
  near(sw, 1200);
  near(sx, 400);
});
