import { createHmac, createPublicKey, timingSafeEqual, verify } from "node:crypto";

export const CHALLENGE_TTL_MS = 5 * 60 * 1000;
const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

function mac(secret, userId, nodeId, expires) {
  return createHmac("sha256", secret).update(`device-link\n${userId}\n${nodeId}\n${expires}`).digest("base64url");
}

export function makeChallenge(secret, userId, nodeId, now = Date.now()) {
  const expires = now + CHALLENGE_TTL_MS;
  return `${expires}.${mac(secret, userId, nodeId, expires)}`;
}

export function checkChallenge(secret, userId, nodeId, challenge, now = Date.now()) {
  const [expiresRaw, sig] = String(challenge ?? "").split(".");
  const expires = Number(expiresRaw);
  if (!Number.isSafeInteger(expires) || !sig || expires < now) return false;
  const expected = Buffer.from(mac(secret, userId, nodeId, expires));
  const given = Buffer.from(sig);
  return expected.length === given.length && timingSafeEqual(expected, given);
}

export function linkMessage(serverUrl, nodeId, challenge) {
  return `commy-device-link:v1\n${serverUrl}\n${nodeId}\n${challenge}`;
}

export function verifyNodeSignature(nodeIdHex, message, signatureHex) {
  if (!/^[0-9a-f]{128}$/i.test(String(signatureHex ?? ""))) return false;
  try {
    const key = createPublicKey({
      key: Buffer.concat([ED25519_SPKI_PREFIX, Buffer.from(nodeIdHex, "hex")]),
      format: "der",
      type: "spki",
    });
    return verify(null, Buffer.from(message), key, Buffer.from(signatureHex, "hex"));
  } catch {
    return false;
  }
}
