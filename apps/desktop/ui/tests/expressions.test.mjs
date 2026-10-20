import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { BUILTIN_EMOTICONS, parseStickerToken, stickerToken, builtinEmoticons, expressionMatches, pickerGroups } = require("../scripts/expressions.js");

test("a sticker message is one sticker token and nothing else", () => {
  const token = stickerToken("wave", "ab12@cd34");
  assert.deepEqual(parseStickerToken(token), { name: "wave", image: "ab12@cd34" });
  assert.deepEqual(parseStickerToken(` ${token}\n`), { name: "wave", image: "ab12@cd34" });
  for (const bad of [`hi ${token}`, `${token}${token}`, "<:wave:ab12@cd34>", "<sticker:Wave:a@b>", "<sticker:wave:ab12>", ""]) {
    assert.equal(parseStickerToken(bad), null, bad);
  }
});

test("built-in emoticons have server-style names, one line each, and no repeats", () => {
  const all = builtinEmoticons();
  assert.ok(all.length >= 30);
  assert.equal(new Set(all.map(e => e.name)).size, all.length);
  for (const { name, text } of all) {
    assert.match(name, /^[a-z0-9_]{2,32}$/);
    assert.ok(text.length > 0 && [...text].length <= 64 && !/[\n\r\t]/.test(text) && text === text.trim(), text);
  }
  assert.ok(BUILTIN_EMOTICONS.every(g => g.group && g.faces.length));
  assert.equal(all.find(e => e.name === "shrug").text, "¯\\_(ツ)_/¯");
});

test("picker search matches names, and emoticon text when asked", () => {
  const faces = [{ name: "shrug", text: "¯\\_(ツ)_/¯" }, { name: "cool", text: "(⌐■_■)" }];
  assert.deepEqual(expressionMatches(faces, "SHR").map(e => e.name), ["shrug"]);
  assert.deepEqual(expressionMatches(faces, "■", "text").map(e => e.name), ["cool"]);
  assert.deepEqual(expressionMatches(faces, "■").map(e => e.name), []);
  assert.equal(expressionMatches(faces, "").length, 2);
});

test("picker groups put this server first and group the rest by server", () => {
  const here = { id: "s1", name: "Home", stickers: [{ name: "wave", image: "a@p" }] };
  const shared = [
    { server: "s1", server_name: "Home", name: "wave", image: "a@p" },
    { server: "s2", server_name: "Art", name: "paint", image: "b@p" },
    { server: "s2", server_name: "Art", name: "brush", image: "c@p" },
  ];
  const groups = pickerGroups(here, shared, "stickers");
  assert.deepEqual(groups.map(g => [g.name, g.local, g.items.map(i => i.name)]), [
    ["Home", true, ["wave"]],
    ["Art", false, ["paint", "brush"]],
  ]);
  assert.deepEqual(groups[1].items[0], { name: "paint", image: "b@p" });
  assert.deepEqual(pickerGroups(here, shared, "stickers", { external: false }).map(g => g.name), ["Home"]);
  assert.deepEqual(pickerGroups(null, shared, "stickers").map(g => g.name), ["Home", "Art"]);
  assert.deepEqual(pickerGroups({ id: "s3", name: "Empty", stickers: [] }, [], "stickers"), []);
});
