import assert from "node:assert/strict";
import { test } from "node:test";
import { keyBytes, keyFromHash, openBackground, openInvite } from "../pages/src/lib/sealed-invite.js";

const KEY = Buffer.from([...Array(32).keys()]).toString("base64url");
const SEALED = "sealed1.BwcHBwcHBwcHBwcHdEjIflUth52--H-g0Dib9cQ5_wCqijSd9uZ65jYZBCaQN0hGsV0";

test("opens an invite sealed by the app", async () => {
  assert.equal((await openInvite(SEALED, KEY))?.name ?? null, "Pixel Pioneers");
});

test("a wrong key, a changed invite or a plain ticket open nothing", async () => {
  assert.equal((await openInvite(SEALED, "B" + KEY.slice(1)))?.name ?? null, null);
  assert.equal((await openInvite(SEALED.slice(0, -2) + "AA", KEY))?.name ?? null, null);
  assert.equal((await openInvite("commy" + "a".repeat(60), KEY))?.name ?? null, null);
  assert.equal((await openInvite(SEALED, "short"))?.name ?? null, null);
});

test("keys come only from a well formed fragment", () => {
  assert.equal(keyFromHash(`#${KEY}`), KEY);
  assert.equal(keyFromHash(""), null);
  assert.equal(keyFromHash("#abc"), null);
  assert.equal(keyFromHash(`#${KEY}x`), null);
});

async function seal(json) {
  const key = await crypto.subtle.importKey("raw", Buffer.from(KEY, "base64url"), "AES-GCM", false, ["encrypt"]);
  const iv = new Uint8Array(12).fill(9);
  const additionalData = new TextEncoder().encode("commy-invite-v1");
  const ct = new Uint8Array(await crypto.subtle.encrypt({ name: "AES-GCM", iv, additionalData }, key, new TextEncoder().encode(JSON.stringify(json))));
  return "sealed1." + Buffer.concat([iv, ct]).toString("base64url");
}

test("the sealed expiry is read, and never or missing told apart", async () => {
  assert.deepEqual(await openInvite(await seal({ t: "x", n: "Club", e: 1800000000 }), KEY), { name: "Club", expires: 1800000000 });
  assert.deepEqual(await openInvite(await seal({ t: "x", n: "Club", p: true }), KEY), { name: "Club", expires: null });
  assert.deepEqual(await openInvite(SEALED, KEY), { name: "Pixel Pioneers", expires: undefined });
  assert.deepEqual(await openInvite(await seal({ t: "x", n: "Club", e: "soon" }), KEY), { name: "Club", expires: undefined });
});

const SEED = Buffer.from([...Array(16).keys()]).toString("base64url");

test("short keys stretch the same way the app does", async () => {
  assert.equal(SEED.length, 22);
  assert.equal(keyFromHash(`#${SEED}`), SEED);
  assert.equal(Buffer.from(await keyBytes(SEED)).toString("base64url"), "DGRsa06xoR5OUNXHMW5Duet9o5gSIQIugDHjgdk8l3k");
  assert.equal(Buffer.from(await keyBytes(KEY)).toString("base64url"), KEY);
});

async function sealWith(key, aad, bytes) {
  const cryptoKey = await crypto.subtle.importKey("raw", await keyBytes(key), "AES-GCM", false, ["encrypt"]);
  const iv = new Uint8Array(12).fill(3);
  const ct = new Uint8Array(await crypto.subtle.encrypt({ name: "AES-GCM", iv, additionalData: new TextEncoder().encode(aad) }, cryptoKey, bytes));
  return "sealed1." + Buffer.concat([iv, ct]).toString("base64url");
}

test("custom links open with their short key", async () => {
  const sealed = await sealWith(SEED, "commy-invite-v1", new TextEncoder().encode(JSON.stringify({ t: "x", n: "Pixel Pioneers", p: true })));
  assert.deepEqual(await openInvite(sealed, SEED), { name: "Pixel Pioneers", expires: null });
});

test("an invite background opens only as an image, with its own purpose", async () => {
  const jpeg = new Uint8Array([0xff, 0xd8, 0xff, 0xe0, 1, 2, 3]);
  const sealed = await sealWith(SEED, "commy-invite-bg-v1", jpeg);
  const blob = await openBackground(sealed, SEED);
  assert.equal(blob?.type, "image/jpeg");
  assert.deepEqual(new Uint8Array(await blob.arrayBuffer()), jpeg);
  assert.equal(await openBackground(sealed, KEY), null, "wrong key");
  assert.equal(await openBackground(await sealWith(SEED, "commy-invite-v1", jpeg), SEED), null, "an invite is not a background");
  assert.equal(await openBackground(await sealWith(SEED, "commy-invite-bg-v1", new TextEncoder().encode("<svg>")), SEED), null, "not an image");
  assert.equal(await openInvite(sealed, SEED), null, "a background is not an invite");
});
