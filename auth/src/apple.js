import { createPrivateKey, sign } from "node:crypto";

const APPLE_AUDIENCE = "https://appleid.apple.com";
const SECRET_LIFETIME_SECONDS = 150 * 24 * 3600;
const RENEW_AFTER_SECONDS = 30 * 24 * 3600;

const base64url = (value) => Buffer.from(typeof value === "string" ? value : JSON.stringify(value)).toString("base64url");

export function signAppleClientSecret({ servicesId, teamId, keyId, privateKey }, now = Math.floor(Date.now() / 1000)) {
  const header = base64url({ alg: "ES256", kid: keyId, typ: "JWT" });
  const claims = base64url({ iss: teamId, iat: now, exp: now + SECRET_LIFETIME_SECONDS, aud: APPLE_AUDIENCE, sub: servicesId });
  const key = createPrivateKey(privateKey);
  const signature = sign("sha256", Buffer.from(`${header}.${claims}`), { key, dsaEncoding: "ieee-p1363" }).toString("base64url");
  return `${header}.${claims}.${signature}`;
}

export function appleClientSecret(web, clock = () => Math.floor(Date.now() / 1000)) {
  let signedAt = clock();
  let secret = signAppleClientSecret(web, signedAt);
  return {
    current() {
      const now = clock();
      if (now - signedAt >= RENEW_AFTER_SECONDS) {
        signedAt = now;
        secret = signAppleClientSecret(web, now);
      }
      return secret;
    },
  };
}
