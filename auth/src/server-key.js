import { createHash, createPrivateKey, createPublicKey, hkdfSync, sign } from "node:crypto";

export const SIGNING_DOMAIN = "commy-server/v1";
export const SERVER_KEY_LABEL = "commy-server-key/v1";
export const LIST_KEY_LABEL = "commy-list-key/v1";
const KEY_ID_BYTES = 8;
const SEED_HEX = /^[0-9a-f]{64}$/i;
const ED25519_PKCS8_PREFIX = Buffer.from("302e020100300506032b657004220420", "hex");
const ED25519_SPKI_PREFIX_BYTES = 12;

export function signedBytes(purpose, origin, payload) {
  return Buffer.concat([Buffer.from(`${SIGNING_DOMAIN}\n${purpose}\n${origin}\n`), Buffer.from(payload)]);
}

export function keyId(publicHex) {
  return createHash("sha256").update(Buffer.from(publicHex, "hex")).digest("hex").slice(0, KEY_ID_BYTES * 2);
}

export function seedFrom({ seedHex, secret, label }) {
  if (seedHex) {
    if (!SEED_HEX.test(seedHex)) throw new Error("a server key must be 64 hex characters");
    return Buffer.from(seedHex, "hex");
  }
  return Buffer.from(hkdfSync("sha256", Buffer.from(secret), Buffer.alloc(0), Buffer.from(label), 32));
}

export function keyFromSeed(seed) {
  const privateKey = createPrivateKey({ key: Buffer.concat([ED25519_PKCS8_PREFIX, seed]), format: "der", type: "pkcs8" });
  const spki = createPublicKey(privateKey).export({ format: "der", type: "spki" });
  const publicHex = spki.subarray(ED25519_SPKI_PREFIX_BYTES).toString("hex");
  const published = { alg: "ed25519", id: keyId(publicHex), public: publicHex };
  return {
    published,
    sign: (purpose, origin, payload) => sign(null, signedBytes(purpose, origin, payload), privateKey).toString("hex"),
  };
}

export function loadServerKey(config, env = process.env) {
  return keyFromSeed(seedFrom({ seedHex: env.AUTH_SERVER_KEY?.trim(), secret: config.secret, label: SERVER_KEY_LABEL }));
}

export function loadListKey(config, env = process.env) {
  return keyFromSeed(seedFrom({ seedHex: env.COMMY_LIST_KEY?.trim(), secret: config.secret, label: LIST_KEY_LABEL }));
}
