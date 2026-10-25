import { createAuthMiddleware } from "better-auth/api";
import { safeReturnPath } from "./security.js";

export const VERIFY_EMAIL_PAGE = "/account/verify-email";
const EXPIRED_CODES = new Set(["TOKEN_EXPIRED"]);

function tokenPayload(token) {
  try {
    const [, payload] = String(token).split(".");
    return JSON.parse(Buffer.from(payload, "base64url").toString("utf8"));
  } catch {
    return null;
  }
}

function isEmailChange(token) {
  return Boolean(tokenPayload(token)?.updateTo);
}

export function verifyEmailResult(location, baseURL) {
  const landing = new URL(location, baseURL);
  const error = landing.searchParams.get("error");
  landing.searchParams.delete("error");
  let result = "confirmed";
  if (error) result = EXPIRED_CODES.has(error.toUpperCase()) ? "expired" : "invalid";
  const page = new URL(VERIFY_EMAIL_PAGE, baseURL);
  page.searchParams.set("result", result);
  page.searchParams.set("next", safeReturnPath(landing.toString(), baseURL));
  return page.toString();
}

export const commyVerifyEmailResult = ({ baseURL }) => ({
  id: "commy-verify-email-result",
  hooks: {
    after: [
      {
        matcher: (ctx) => ctx.path === "/verify-email",
        handler: createAuthMiddleware(async (ctx) => {
          if (!ctx.query?.callbackURL || isEmailChange(ctx.query.token)) return;
          const location = ctx.context.responseHeaders?.get("location");
          if (!location) return;
          throw ctx.redirect(verifyEmailResult(location, baseURL));
        }),
      },
    ],
  },
});
