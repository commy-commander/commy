import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { STOCK_BANNERS, stockBannerSrc, stockBannerTiles } = require("../scripts/stock-banners.js");
const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const app = read("../scripts/app.js");
const classic = read("../styles/styles.css");
const float = read("../styles/studio-float.css");

test("twelve stock banners, each with a name of its own and a drawing in the app", () => {
  assert.equal(STOCK_BANNERS.length, 12);
  assert.equal(new Set(STOCK_BANNERS.map(banner => banner.id)).size, 12);
  assert.equal(new Set(STOCK_BANNERS.map(banner => banner.name)).size, 12);
  for (const banner of STOCK_BANNERS) {
    assert.match(banner.id, /^[a-z]+(-[a-z]+)*$/);
    assert.equal(stockBannerSrc(banner.id), `images/banners/${banner.id}.svg`);
    assert.ok(existsSync(new URL(`../${stockBannerSrc(banner.id)}`, import.meta.url)), banner.id);
  }
  assert.equal(stockBannerSrc("nope"), null);
  const files = readdirSync(new URL("../images/banners/", import.meta.url)).sort();
  assert.deepEqual(files, STOCK_BANNERS.map(banner => `${banner.id}.svg`).sort());
});

test("each drawing is a plain picture three times as wide as tall, with its name and nothing from outside", () => {
  for (const banner of STOCK_BANNERS) {
    const art = read(`../${stockBannerSrc(banner.id)}`);
    assert.ok(art.startsWith('<svg xmlns="http://www.w3.org/2000/svg" width="960" height="320" viewBox="0 0 960 320" preserveAspectRatio="xMidYMid slice">'), banner.id);
    assert.ok(art.includes(`<title>${banner.name}</title>`), banner.id);
    assert.equal(art.includes("<!--"), false, banner.id);
    assert.doesNotMatch(art, /<script|<image|<foreignObject|href=|url\((?!#)/, banner.id);
    assert.ok(art.length < 20000, banner.id);
  }
});

test("the tiles are keys named for their banner, and a press hands over its picture", () => {
  const made = [];
  const doc = { createElement: tag => { const node = { tag, children: [], attrs: {}, setAttribute(name, value) { this.attrs[name] = value; }, appendChild(child) { this.children.push(child); } }; made.push(node); return node; } };
  const picked = [];
  const tiles = stockBannerTiles(doc, src => picked.push(src));
  assert.equal(tiles.length, 12);
  assert.equal(tiles[0].tag, "button");
  assert.equal(tiles[0].type, "button");
  assert.equal(tiles[0].className, "banner-tile");
  assert.equal(tiles[0].title, "Night peaks");
  assert.equal(tiles[0].attrs["aria-label"], "Night peaks");
  assert.equal(tiles[0].children[0].tag, "img");
  assert.equal(tiles[0].children[0].src, "images/banners/night-peaks.svg");
  assert.equal(tiles[0].children[0].alt, "");
  tiles[4].onclick();
  assert.deepEqual(picked, ["images/banners/orbit.svg"]);
});

test("every place that takes a banner opens the same menu: the stock banners, then your own picture", () => {
  const menu = html.slice(html.indexOf('<div class="pop-menu banner-menu hidden" id="banner-menu">'), html.indexOf('<input type="file" id="photo-file"'));
  assert.ok(menu.indexOf('id="banner-tiles"') > 0);
  assert.ok(menu.indexOf('<button id="banner-own">Upload a Picture</button>') > menu.indexOf('id="banner-tiles"'));
  assert.ok(html.indexOf('<script src="scripts/stock-banners.js"></script>') < html.indexOf('<script src="scripts/app.js"></script>'));
  for (const title of ["Choose a profile banner", "Choose a channel banner", "Choose a role banner", "Choose a server banner", "Choose an invite background"]) {
    const asked = app.split(`"${title}"`).length - 1;
    assert.ok(asked >= 1, title);
    assert.equal(app.split(`openBannerMenu(e.currentTarget, "${title}", `).length + app.split(`openBannerMenu(anchor, "${title}", `).length - 2, asked, title);
  }
  assert.ok(app.includes('const src = stock || (await chooseImage(title, $("server-image-file")));'));
  assert.ok(app.includes('$("banner-own").onclick = () => bannerMenuUse && bannerMenuUse(null);'));
  assert.equal(app.split('"avatar-menu", "banner-menu", ').length - 1, 2);
});

test("the menu shows the banners three across, above Settings, in both looks of the pop-up", () => {
  assert.ok(classic.includes("#avatar-menu, #banner-menu { z-index: 180; }"));
  assert.ok(classic.includes(".banner-tiles { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 6px; padding: 4px 4px 8px; }"));
  assert.ok(classic.includes(".pop-menu .banner-tile > img { display: block; width: 100%; aspect-ratio: 5 / 2; object-fit: cover; }"));
  assert.ok(float.includes('html[data-look="studio"]:not([data-fit="slim"]) .banner-menu { width: 376px; }'));
  assert.ok(float.includes('html[data-look="studio"]:not([data-fit="slim"]) .pop-menu .banner-tile { min-height: 0; padding: 0; border-radius: 12px;'));
});
