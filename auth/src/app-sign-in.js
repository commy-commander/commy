import * as z from "zod";
import { APIError, createAuthEndpoint, getSessionFromCtx, signInSocial } from "better-auth/api";
import { deleteSessionCookie } from "better-auth/cookies";
import { createHash, randomBytes, timingSafeEqual } from "node:crypto";
import { ACCESS_COOKIE, accessToken, normalizeAccessCode } from "./access.js";
import { CAPTCHA_PATH } from "./config.js";

export const APP_RETURN_URL = "commy://signed-in";
export const SHEET_PROVIDERS = ["google", "twitch"];
const HANDOFF_SECONDS = 600;
const ACCESS_COOKIE_SECONDS = 600;
const HANDOFF_PREFIX = "app-handoff-";
const HANDOFF_ID = /^[A-Za-z0-9_-]{32}$/;

export function androidOrigin(fingerprint) {
  return `android:apk-key-hash:${Buffer.from(fingerprint.replace(/:/g, ""), "hex").toString("base64url")}`;
}

export function appleAppSiteAssociation(iosAppIds) {
  return { webcredentials: { apps: iosAppIds } };
}

export function assetLinks(android) {
  return [
    {
      relation: ["delegate_permission/common.get_login_creds"],
      target: { namespace: "android_app", package_name: android.package, sha256_cert_fingerprints: android.fingerprints },
    },
  ];
}

export const APP_ORIGINS = ["tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"];
const TURNSTILE_SCRIPT = "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";

function sameText(a, b) {
  const left = Buffer.from(a);
  const right = Buffer.from(b);
  return left.length === right.length && timingSafeEqual(left, right);
}

export function captchaPage(siteKey) {
  const key = JSON.stringify(siteKey).replace(/</g, "\\u003c");
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Commy check</title>
<style>html,body{margin:0;background:transparent;display:flex;justify-content:center;overflow:hidden}</style>
<script src="${TURNSTILE_SCRIPT}" async defer onload="start()"></script>
<script>
function tell(message) { parent.postMessage(Object.assign({ type: "commy-captcha" }, message), "*"); }
function start() {
  turnstile.render("#check", {
    sitekey: ${key},
    theme: "dark",
    callback: function (token) { tell({ token: token }); },
    "expired-callback": function () { tell({ expired: true }); turnstile.reset("#check"); },
    "error-callback": function () { tell({ failed: true }); },
  });
}
</script>
</head>
<body><div id="check"></div></body>
</html>`;
}

function captchaHeaders() {
  return {
    "content-type": "text/html; charset=utf-8",
    "cache-control": "no-store",
    "content-security-policy": `frame-ancestors ${APP_ORIGINS.join(" ")}`,
  };
}

const sha256 = (value) => createHash("sha256").update(value).digest("base64url");

async function readHandoff(ctx, id) {
  if (!HANDOFF_ID.test(id ?? "")) return null;
  const row = await ctx.context.internalAdapter.findVerificationValue(`${HANDOFF_PREFIX}${id}`);
  if (!row || new Date(row.expiresAt) < new Date()) return null;
  try {
    return JSON.parse(row.value);
  } catch {
    return null;
  }
}

async function writeHandoff(ctx, id, value) {
  const identifier = `${HANDOFF_PREFIX}${id}`;
  await ctx.context.internalAdapter.deleteVerificationByIdentifier(identifier);
  await ctx.context.internalAdapter.createVerificationValue({
    identifier,
    value: JSON.stringify(value),
    expiresAt: new Date(Date.now() + HANDOFF_SECONDS * 1000),
  });
}

function backToApp(id, code) {
  const url = new URL(APP_RETURN_URL);
  url.searchParams.set("handoff", id);
  url.searchParams.set("code", code);
  return url.toString();
}

export function backPage(url) {
  const target = JSON.stringify(url).replace(/</g, "\\u003c");
  const href = url.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Commy</title>
<style>html,body{margin:0;min-height:100vh;background:#0b0d17;color:#f2efe6;font:16px system-ui,sans-serif;display:grid;place-items:center;text-align:center}a{display:inline-block;margin-top:16px;padding:12px 20px;border-radius:10px;background:#ff4d4d;color:#0b0d17;font-weight:600;text-decoration:none}</style>
</head>
<body>
<div><p>Going back to Commy…</p><a id="back" href="${href}">Back to Commy</a></div>
<script>location.replace(${target});</script>
</body>
</html>`;
}

function sheetStartBody({ provider, back, adultConfirmed }) {
  return {
    provider,
    callbackURL: back,
    errorCallbackURL: back,
    ...(adultConfirmed ? { additionalData: { adultConfirmed: true } } : {}),
  };
}

export const commyAppSignIn = ({ accessCode, turnstileSiteKey }) => ({
  id: "commy-app-sign-in",
  endpoints: {
    checkAccessCode: createAuthEndpoint(
      "/commy/access",
      { method: "POST", body: z.object({ code: z.string().max(200) }) },
      async (ctx) => {
        if (!accessCode) return ctx.json({ token: null });
        if (!sameText(normalizeAccessCode(ctx.body.code), accessCode)) {
          throw new APIError("FORBIDDEN", { code: "WRONG_ACCESS_CODE", message: "That access code is not right." });
        }
        return ctx.json({ token: accessToken(accessCode) });
      },
    ),
    startSheetSignIn: createAuthEndpoint(
      "/commy/app-sign-in/start",
      {
        method: "POST",
        body: z.object({
          provider: z.enum(SHEET_PROVIDERS),
          challenge: z.string().regex(/^[A-Za-z0-9_-]{43}$/),
          adultConfirmed: z.boolean().optional(),
          accessToken: z.string().max(200).optional(),
        }),
      },
      async (ctx) => {
        if (!ctx.context.socialProviders.some((p) => p.id === ctx.body.provider)) {
          throw new APIError("NOT_FOUND", { code: "PROVIDER_NOT_FOUND", message: "This account server does not offer that sign-in." });
        }
        const id = randomBytes(24).toString("base64url");
        await writeHandoff(ctx, id, {
          provider: ctx.body.provider,
          challenge: ctx.body.challenge,
          adultConfirmed: ctx.body.adultConfirmed === true,
          access: ctx.body.accessToken ?? null,
          opened: false,
        });
        const go = new URL(`${ctx.context.baseURL}/commy/app-sign-in/go`);
        go.searchParams.set("handoff", id);
        return ctx.json({ url: go.toString(), returnUrl: APP_RETURN_URL });
      },
    ),
    openSheetSignIn: createAuthEndpoint("/commy/app-sign-in/go", { method: "GET", query: z.object({ handoff: z.string().max(64) }) }, async (ctx) => {
      const handoff = await readHandoff(ctx, ctx.query.handoff);
      if (!handoff || handoff.opened) return new Response("This sign-in link has expired. Close it and try again.", { status: 410 });
      await writeHandoff(ctx, ctx.query.handoff, { ...handoff, opened: true });
      const back = `${ctx.context.baseURL}/commy/app-sign-in/back?handoff=${ctx.query.handoff}`;
      const started = await signInSocial()({
        ...ctx,
        method: "POST",
        body: sheetStartBody({ provider: handoff.provider, back, adultConfirmed: handoff.adultConfirmed }),
        asResponse: false,
        returnHeaders: true,
      });
      const target = started.response?.url;
      if (!target) return new Response("Sign-in could not start. Close this and try again.", { status: 502 });
      const headers = new Headers({ location: target, "cache-control": "no-store", "referrer-policy": "no-referrer" });
      for (const cookie of started.headers.getSetCookie()) headers.append("set-cookie", cookie);
      if (handoff.access) {
        const secure = ctx.context.baseURL.startsWith("https://") ? "; Secure" : "";
        headers.append("set-cookie", `${ACCESS_COOKIE}=${handoff.access}; Path=/; Max-Age=${ACCESS_COOKIE_SECONDS}; HttpOnly; SameSite=Lax${secure}`);
      }
      return new Response(null, { status: 302, headers });
    }),
    returnFromSheet: createAuthEndpoint(
      "/commy/app-sign-in/back",
      { method: "GET", query: z.object({ handoff: z.string().max(64), error: z.string().max(200).optional() }) },
      async (ctx) => {
        const id = ctx.query.handoff;
        const handoff = await readHandoff(ctx, id);
        if (!handoff || handoff.code) return new Response("This sign-in has expired. Close it and try again.", { status: 410 });
        const signedIn = ctx.query.error ? null : await getSessionFromCtx(ctx, { disableRefresh: true });
        const result = signedIn?.session?.token
          ? { token: signedIn.session.token }
          : { error: (ctx.query.error || "no_session").slice(0, 80) };
        const code = randomBytes(24).toString("base64url");
        await writeHandoff(ctx, id, { ...handoff, ...result, code: sha256(code) });
        if (signedIn) deleteSessionCookie(ctx, true);
        const headers = { "content-type": "text/html; charset=utf-8", "cache-control": "no-store", "referrer-policy": "no-referrer" };
        return new Response(backPage(backToApp(id, code)), { status: 200, headers });
      },
    ),
    finishSheetSignIn: createAuthEndpoint(
      "/commy/app-sign-in/finish",
      { method: "POST", body: z.object({ handoff: z.string().max(64), verifier: z.string().min(43).max(128), code: z.string().max(64) }) },
      async (ctx) => {
        const handoff = await readHandoff(ctx, ctx.body.handoff);
        const matches = handoff && sameText(sha256(ctx.body.verifier), handoff.challenge) && handoff.code && sameText(sha256(ctx.body.code), handoff.code);
        if (!matches) {
          throw new APIError("NOT_FOUND", { code: "HANDOFF_NOT_FOUND", message: "That sign-in has expired. Start again." });
        }
        await ctx.context.internalAdapter.deleteVerificationByIdentifier(`${HANDOFF_PREFIX}${ctx.body.handoff}`);
        if (handoff.error) {
          throw new APIError("FORBIDDEN", { code: handoff.error.toUpperCase().replace(/[^A-Z0-9_]/g, "_"), message: handoff.error });
        }
        return ctx.json({ token: handoff.token });
      },
    ),
    appCaptcha: createAuthEndpoint(CAPTCHA_PATH, { method: "GET" }, async () => {
      if (!turnstileSiteKey) return new Response("Not found", { status: 404 });
      return new Response(captchaPage(turnstileSiteKey), { status: 200, headers: captchaHeaders() });
    }),
  },
});
