import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { STUDIO_FACES, STUDIO_BOT_FACE, STUDIO_FACE_COLOURS, STUDIO_FACE_TINT, STUDIO_FACE_SPRITE, STUDIO_FACE_SHARED, STUDIO_FACE_ART, studioFaceFor, studioFaceColour, studioFaceSprite } = require("../scripts/studio-faces.js");
const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const app = read("scripts", "app.js");
const html = read("index.html");
const parts = read("styles", "studio-parts.css");
const tokens = read("styles", "tokens.css");

const MOON_CHEEK = '<ellipse cx="14.6" cy="23.4" rx="1.6" ry="1.1" style="fill:#f08f7d" opacity=".55"/>';

test("a person with no picture always gets the same face, on every device", () => {
  for (const id of ["a", "user-1", "9f3c2d", "0"]) {
    assert.equal(studioFaceFor(id), studioFaceFor(id));
    assert.ok(STUDIO_FACES.includes(studioFaceFor(id)));
  }
  assert.equal(studioFaceFor(12), studioFaceFor("12"));
});

test("the set is ten faces in a fixed order, and the rank mark is not dealt out", () => {
  assert.deepEqual(STUDIO_FACES, ["fox", "owl", "finch", "visor", "moon", "frog", "cat", "bear", "rabbit", "planet"]);
  assert.ok(!STUDIO_FACES.includes("helm"));
});

test("faces are shared out evenly", () => {
  const faces = Object.fromEntries(STUDIO_FACES.map(name => [name, 0]));
  for (let i = 0; i < 12000; i++) faces[studioFaceFor(`person-${i}`)]++;
  for (const name of STUDIO_FACES) assert.ok(faces[name] > 1000 && faces[name] < 1400, `${name}: ${faces[name]}`);
});

test("each face wears its colour from the style guide's default avatars", () => {
  assert.deepEqual(STUDIO_FACE_TINT, { fox: "ferris", owl: "nova", finch: "pip", visor: "ash", moon: "lumen", frog: "dot", cat: "lumen", bear: "ferris", rabbit: "nova", planet: "dot", bot: "bot" });
  for (let i = 0; i < 500; i++) assert.equal(studioFaceColour(`person-${i}`), STUDIO_FACE_TINT[studioFaceFor(`person-${i}`)]);
  assert.equal(studioFaceColour("person-1", true), "bot");
  assert.match(tokens, /--role-bot:/);
});

test("a face takes one of the six avatar colours from the tokens, the same for a person every time", () => {
  assert.deepEqual(STUDIO_FACE_COLOURS, ["ferris", "nova", "pip", "ash", "lumen", "dot"]);
  for (const name of STUDIO_FACE_COLOURS) assert.match(tokens, new RegExp(`--role-${name}:`));
  for (const tint of Object.values(STUDIO_FACE_TINT).filter(tint => tint !== "bot")) assert.ok(STUDIO_FACE_COLOURS.includes(tint), tint);
  assert.equal(studioFaceColour("9f3c2d"), studioFaceColour("9f3c2d"));
  assert.match(read("scripts", "studio-faces.js"), /svg\.style\.setProperty\("--c", `var\(--role-\$\{studioFaceColour\(id, bot\)\}\)`\);/);
  assert.doesNotMatch(read("scripts", "studio-faces.js"), /--tint|personTint/);
});

test("the robot is kept for bots, and a bot never gets anything else", () => {
  assert.ok(!STUDIO_FACES.includes(STUDIO_BOT_FACE));
  for (let i = 0; i < 500; i++) {
    assert.notEqual(studioFaceFor(`person-${i}`), STUDIO_BOT_FACE);
    assert.equal(studioFaceFor(`person-${i}`, true), STUDIO_BOT_FACE);
  }
});

test("every face has a drawing, in one colour on the cap, and only the moon's cheek has a colour of its own", () => {
  for (const name of [...STUDIO_FACES, STUDIO_BOT_FACE]) {
    const art = STUDIO_FACE_ART[name];
    assert.ok(art, name);
    assert.ok(art.startsWith('<use href="#sf-cap"/>'), name);
    assert.match(art, /var\(--c\)/);
    assert.doesNotMatch(name === "moon" ? art.replace(MOON_CHEEK, "") : art, /#[0-9a-f]{3,8}\b|rgb\(/i);
  }
  assert.equal(STUDIO_FACE_ART.moon.split(MOON_CHEEK).length, 2);
  assert.ok(STUDIO_FACE_ART.moon.includes('<path d="M23.5 8.57A12.2 12.2 0 1 0 31.84 27.07 10.2 10.2 0 0 1 23.5 8.57Z" style="fill:var(--c)"/>'));
  assert.ok(STUDIO_FACE_ART.moon.includes('<path d="M13.1 18.4q1.9 2 3.8 0" style="fill:none;stroke:var(--a-ground);stroke-width:1.7;stroke-linecap:round"/>'));
  assert.ok(!STUDIO_FACE_ART.moon.includes("#sf-eye"));
  assert.equal(Object.keys(STUDIO_FACE_ART).length, STUDIO_FACES.length + 1);
  assert.match(STUDIO_FACE_SHARED, /color-mix\(in srgb, var\(--c\) 24%, var\(--a-ground\)\)/);
});

test("the sprite holds one symbol per face and is mounted once", () => {
  const sprite = studioFaceSprite();
  assert.ok(sprite.startsWith(`<svg id="${STUDIO_FACE_SPRITE}"`));
  for (const name of [...STUDIO_FACES, STUDIO_BOT_FACE]) assert.equal(sprite.split(`<symbol id="sf-${name}"`).length, 2);
  assert.match(read("scripts", "studio-faces.js"), /if \(!doc\.body \|\| doc\.getElementById\(STUDIO_FACE_SPRITE\)\) return;/);
});

test("the faces replace initials only in the new look, and a picture still wins", () => {
  assert.match(app, /if \(!studioLook\(\)\) return el\("span", "avatar-face", initials\(name \|\| "\?"\)\);/);
  assert.match(app, /return studioFaceNode\(document, userId, knownBot\(userId\)\);/);
  assert.match(app, /node\.classList\.remove\("face-avatar"\);\s+node\.classList\.add\("image-avatar"\);/);
  assert.match(app, /node\.classList\.remove\("emoji-avatar", "image-avatar", "face-avatar"\);/);
  assert.ok(html.indexOf('<script src="scripts/studio-faces.js"></script>') > 0);
  assert.ok(html.indexOf('<script src="scripts/studio-faces.js"></script>') < html.indexOf('<script src="scripts/app.js"></script>'));
  assert.match(parts, /html\[data-look="studio"\] \.avatar > svg\.avatar-face \{[^}]*border-radius: 50%;/);
  assert.match(parts, /html\[data-look="studio"\] \.avatar\.face-avatar \{ background: none; \}/);
});
