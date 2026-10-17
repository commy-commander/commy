// Optional access code for a private preview. With ACCESS_CODE set, the website
// asks for the code before it shows downloads or the sign-up form, and stores
// proof of it in the commy_access cookie. The auth server refuses to create an
// account, by any sign-up route, unless the request carries that cookie.
// The website (commy-web lib/access.ts) derives the same value; keep them in step.
import { createHmac, timingSafeEqual } from "node:crypto";

export const ACCESS_COOKIE = "commy_access";

// Codes are typed by people, so spacing and case never matter
export function normalizeAccessCode(code) {
  return String(code ?? "").trim().toLowerCase();
}

// The cookie value: changing the code makes every earlier cookie stop working
export function accessToken(code) {
  return createHmac("sha256", normalizeAccessCode(code)).update("commy-access-v1").digest("base64url");
}

function readCookie(header, name) {
  for (const part of (header ?? "").split(";")) {
    const [key, ...rest] = part.trim().split("=");
    if (key === name) return rest.join("=");
  }
  return null;
}

export function hasAccess(cookieHeader, code) {
  if (!code) return true;
  const given = Buffer.from(readCookie(cookieHeader, ACCESS_COOKIE) ?? "");
  const expected = Buffer.from(accessToken(code));
  return given.length === expected.length && timingSafeEqual(given, expected);
}
