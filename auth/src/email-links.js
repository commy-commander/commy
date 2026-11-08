import { createHmac, hkdfSync, timingSafeEqual } from "node:crypto";

export const PREFERENCES_PATH = "/commy/email/preferences";
const TOKEN_CHARS = 32;

function linkKey(secret) {
  return Buffer.from(hkdfSync("sha256", secret, "commy-email-links-v1", "email preference links", 32));
}

function normalizedAddress(email) {
  return String(email ?? "").trim().toLowerCase();
}

export function emailLinkSigner(secret) {
  const key = linkKey(secret);
  const tokenFor = (address) => createHmac("sha256", key).update(address).digest("base64url").slice(0, TOKEN_CHARS);

  return {
    linkQuery(email) {
      const address = normalizedAddress(email);
      return new URLSearchParams({ a: Buffer.from(address).toString("base64url"), t: tokenFor(address) }).toString();
    },
    verify(encodedAddress, token) {
      const address = normalizedAddress(Buffer.from(String(encodedAddress ?? ""), "base64url").toString("utf8"));
      if (!address.includes("@")) return null;
      const expected = Buffer.from(tokenFor(address));
      const given = Buffer.from(String(token ?? ""));
      return given.length === expected.length && timingSafeEqual(given, expected) ? address : null;
    },
  };
}

export function preferencesUrlMaker({ secret, baseURL, basePath }) {
  const signer = emailLinkSigner(secret);
  return (email) => `${baseURL}${basePath}${PREFERENCES_PATH}?${signer.linkQuery(email)}`;
}
