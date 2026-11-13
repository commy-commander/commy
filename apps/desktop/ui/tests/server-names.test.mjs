import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { serverNameParts, serverShownName } = require("../scripts/server-names.js");

test("a server name shows without its emoji codes", () => {
  assert.equal(serverShownName("Gabagool Industries :pepodancels:"), "Gabagool Industries");
  assert.equal(serverShownName(":pepodancels: Gabagool  Industries"), "Gabagool Industries");
  assert.equal(serverShownName("Pixel :heart: Club"), "Pixel Club");
});

test("a name made only of an emoji code keeps the word", () => {
  assert.equal(serverShownName(":pepodancels:"), "pepodancels");
});

test("times and plain colons stay as they are", () => {
  assert.equal(serverShownName("Night shift 10:30:45"), "Night shift 10:30:45");
  assert.equal(serverShownName("Team: Alpha"), "Team: Alpha");
  assert.equal(serverShownName(""), "");
});

test("the server's own emoji become pictures and unknown codes drop out", () => {
  const dance = { name: "pepodancels", image: "a@b" };
  assert.deepEqual(serverNameParts("Gabagool :pepodancels: :other:", [dance]), ["Gabagool ", { emoji: dance }]);
  assert.deepEqual(serverNameParts(":pepodancels: Gabagool", [dance]), [{ emoji: dance }, " Gabagool"]);
  assert.deepEqual(serverNameParts("Gabagool :other: Industries", []), ["Gabagool Industries"]);
});

test("a name of only unknown codes falls back to the words", () => {
  assert.deepEqual(serverNameParts(":pepodancels:", []), ["pepodancels"]);
  assert.deepEqual(serverNameParts(":pepodancels:", null), ["pepodancels"]);
});
