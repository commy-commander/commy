import * as z from "zod";
import { APIError, createAuthEndpoint } from "better-auth/api";
import { timingSafeEqual } from "node:crypto";
import { accessToken, normalizeAccessCode } from "./access.js";
import { CAPTCHA_PATH } from "./config.js";

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
    appCaptcha: createAuthEndpoint(CAPTCHA_PATH, { method: "GET" }, async () => {
      if (!turnstileSiteKey) return new Response("Not found", { status: 404 });
      return new Response(captchaPage(turnstileSiteKey), { status: 200, headers: captchaHeaders() });
    }),
  },
});
