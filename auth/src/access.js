import { createHmac, timingSafeEqual } from "node:crypto";

export const ACCESS_COOKIE = "commy_access";

export function normalizeAccessCode(code) {
  return String(code ?? "").trim().toLowerCase();
}

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
