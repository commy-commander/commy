import assert from "node:assert/strict";
import { generateKeyPairSync, sign } from "node:crypto";
import { test } from "node:test";
import { networkTag, sameNetwork } from "../src/device-requests.js";
import { CHALLENGE_TTL_MS, checkChallenge, linkMessage, makeChallenge, verifyNodeSignature } from "../src/proof.js";

const SECRET = "s".repeat(32);
const NODE = "ab".repeat(32);

function keypair() {
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  return {
    nodeId: publicKey.export({ format: "der", type: "spki" }).subarray(12).toString("hex"),
    sign: (m) => sign(null, Buffer.from(m), privateKey).toString("hex"),
  };
}

test("challenges are bound to user, node and time", () => {
  const now = 1_700_000_000_000;
  const c = makeChallenge(SECRET, "u1", NODE, now);
  assert.equal(checkChallenge(SECRET, "u1", NODE, c, now + 1000), true);
  assert.equal(checkChallenge(SECRET, "u2", NODE, c, now), false);
  assert.equal(checkChallenge(SECRET, "u1", "cd".repeat(32), c, now), false);
  assert.equal(checkChallenge("other".repeat(8), "u1", NODE, c, now), false);
  assert.equal(checkChallenge(SECRET, "u1", NODE, c, now + CHALLENGE_TTL_MS + 1), false);
});

test("tampered or malformed challenges fail closed", () => {
  const now = Date.now();
  const c = makeChallenge(SECRET, "u1", NODE, now);
  const [exp, mac] = c.split(".");
  assert.equal(checkChallenge(SECRET, "u1", NODE, `${Number(exp) + 60000}.${mac}`, now), false);
  for (const bad of ["", "abc", "1.2.3", `${exp}.`, `x.${mac}`, undefined, null]) {
    assert.equal(checkChallenge(SECRET, "u1", NODE, bad, now), false);
  }
});

test("linkMessage binds the server URL", () => {
  assert.notEqual(linkMessage("https://a.test", NODE, "c"), linkMessage("https://b.test", NODE, "c"));
  assert.equal(linkMessage("https://a.test", NODE, "c"), `commy-device-link:v1\nhttps://a.test\n${NODE}\nc`);
});

test("node signatures verify only for the matching key and message", () => {
  const k = keypair();
  const msg = linkMessage("https://a.test", k.nodeId, "c");
  const sig = k.sign(msg);
  assert.equal(verifyNodeSignature(k.nodeId, msg, sig), true);
  assert.equal(verifyNodeSignature(k.nodeId, msg + "x", sig), false);
  assert.equal(verifyNodeSignature(keypair().nodeId, msg, sig), false);
  assert.equal(verifyNodeSignature(k.nodeId, msg, sig.toUpperCase()), true);
  assert.equal(verifyNodeSignature(k.nodeId, msg, "00"), false);
  assert.equal(verifyNodeSignature(k.nodeId, msg, undefined), false);
  assert.equal(verifyNodeSignature("zz", msg, sig), false);
});

test("network tags hide the IP and compare equal per network", () => {
  const a = networkTag(SECRET, "203.0.113.5");
  assert.equal(a.includes("203"), false);
  assert.equal(a, networkTag(SECRET, "203.0.113.5"));
  assert.notEqual(a, networkTag(SECRET, "203.0.113.6"));
  assert.notEqual(a, networkTag("t".repeat(32), "203.0.113.5"));
  assert.equal(networkTag(SECRET, null), "");
  assert.equal(sameNetwork(a, a), true);
  assert.equal(sameNetwork(a, networkTag(SECRET, "198.51.100.1")), false);
  assert.equal(sameNetwork("", a), null);
  assert.equal(sameNetwork(a, ""), null);
});
