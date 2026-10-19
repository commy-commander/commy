import assert from "node:assert/strict";
import { test } from "node:test";
import { socialProviders } from "../src/auth.js";
import { discoveryDocument, loadConfig } from "../src/config.js";
import { DEVICE_LABEL, dropDeviceNames, normalizeNodeId, toPublicDevice } from "../src/devices.js";
import { createMailer, escapeHtml, renderEmail } from "../src/mailer.js";

const base = {
  BETTER_AUTH_SECRET: "x".repeat(32),
  DATABASE_URL: "postgres://u:p@db:5432/auth",
  BETTER_AUTH_URL: "https://chat.example.org/",
};

test("loadConfig applies defaults and strips trailing slash", () => {
  const c = loadConfig(base);
  assert.equal(c.baseURL, "https://chat.example.org");
  assert.equal(c.port, 3001);
  assert.equal(c.basePath, "/api/auth");
  assert.deepEqual(c.trustedOrigins, ["https://chat.example.org"]);
  assert.deepEqual(c.desktopClientIds, ["commy-desktop"]);
  assert.equal(c.allowSignup, true);
  assert.equal(c.google, null);
  assert.equal(c.smtp, null);
});

test("loadConfig reports every invalid value at once", () => {
  assert.throws(
    () => loadConfig({ BETTER_AUTH_SECRET: "short", DATABASE_URL: "mysql://x", BETTER_AUTH_URL: "nope", PORT: "99999" }),
    (err) => ["BETTER_AUTH_SECRET", "DATABASE_URL", "BETTER_AUTH_URL", "PORT"].every((k) => err.message.includes(k)),
  );
});

test("loadConfig enables providers only when fully configured", () => {
  assert.equal(loadConfig({ ...base, GOOGLE_CLIENT_ID: "id" }).google, null);
  const c = loadConfig({
    ...base,
    GOOGLE_CLIENT_ID: "id",
    GOOGLE_CLIENT_SECRET: "sec",
    SMTP_URL: "smtp://mail:25",
    TRUSTED_ORIGINS: "https://a.test, https://chat.example.org,,",
    DEVICE_CLIENT_IDS: "one,two",
    ALLOW_SIGNUP: "false",
  });
  assert.deepEqual(c.google, { clientId: "id", clientSecret: "sec" });
  assert.equal(c.smtp.transport, "smtp://mail:25");
  assert.equal(c.twitch, null);
  assert.deepEqual(c.trustedOrigins, ["https://chat.example.org", "https://a.test"]);
  assert.deepEqual(c.desktopClientIds, ["one", "two"]);
  assert.equal(c.allowSignup, false);
});

test("discovery document points at the public URL", () => {
  const d = discoveryDocument(loadConfig({ ...base, SMTP_URL: "smtp://m" }));
  assert.equal(d.api, "https://chat.example.org/api/auth");
  assert.equal(d.deviceAuthorization.clientId, "commy-desktop");
  assert.equal(d.deviceAuthorization.verificationUri, "https://chat.example.org/device");
  assert.equal(d.deviceAuthorization.tokenEndpoint, "https://chat.example.org/api/auth/device/token");
  assert.deepEqual(d.providers, {
    google: false,
    twitch: false,
    magicLink: true,
    emailVerification: true,
    passwordReset: true,
    emailPassword: true,
    passkey: true,
  });
  assert.equal(d.features.changeEmail, true);
  assert.equal(d.turnstileSiteKey, null);
  assert.equal(d.endpoints.linkChallenge, "https://chat.example.org/api/auth/commy/devices/challenge");
});

test("normalizeNodeId accepts only 32 byte hex keys", () => {
  const id = "AB".repeat(32);
  assert.equal(normalizeNodeId(` ${id} `), id.toLowerCase());
  assert.equal(normalizeNodeId("ab".repeat(31)), null);
  assert.equal(normalizeNodeId("zz".repeat(32)), null);
  assert.equal(normalizeNodeId(undefined), null);
});

test("toPublicDevice drops the owning user id and any stored name", () => {
  const out = toPublicDevice({ id: "1", nodeId: "n", name: "DESKTOP-ABC123", userId: "secret", createdAt: 1, lastSeenAt: 2 });
  assert.equal("userId" in out, false);
  assert.equal(out.name, DEVICE_LABEL);
});

test("dropDeviceNames removes the old name column", async () => {
  const queries = [];
  await dropDeviceNames({ query: async (q) => queries.push(q) });
  assert.deepEqual(queries, ['ALTER TABLE IF EXISTS "linkedDevice" DROP COLUMN IF EXISTS "name"']);
});

test("emails escape user controlled values", () => {
  assert.equal(escapeHtml(`<a href="x">'&`), "&lt;a href=&quot;x&quot;&gt;&#39;&amp;");
  const { html, text } = renderEmail({ instanceName: "<b>", heading: "Hi", body: "b", actionLabel: "Go", url: "https://x/?a=1&b=2" });
  assert.ok(html.includes("&lt;b&gt;"));
  assert.ok(html.includes("a=1&amp;b=2"));
  assert.ok(text.includes("https://x/?a=1&b=2"));
});

test("emails carry their own icon and load nothing from other servers", () => {
  const { html, text } = renderEmail({
    instanceName: "Commy",
    siteUrl: "https://commy.chat",
    heading: "Verify your email",
    body: "Confirm this address.",
    actionLabel: "Verify email",
    url: "https://commy.chat/api/auth/verify-email?token=t",
  });
  assert.ok(html.includes('src="cid:'));
  const remote = [...html.matchAll(/(?:src|href)="(https?:[^"]+)"/g)].map((m) => m[1]);
  assert.deepEqual([...new Set(remote)], ["https://commy.chat/api/auth/verify-email?token=t"]);
  assert.equal(/<link|@import|url\(/.test(html), false);
  assert.match(text, /Verify email:\nhttps:\/\/commy\.chat\/api\/auth\/verify-email\?token=t/);
  assert.match(text, /Sent by Commy from commy\.chat/);
});

test("notice emails without a link render no button", () => {
  const { html, text } = renderEmail({
    instanceName: "Commy",
    siteUrl: "https://commy.chat",
    heading: "Your password changed",
    body: ["First.", "Second."],
    note: "If this was you, there is nothing to do.",
  });
  assert.equal(html.includes("If the button"), false);
  assert.ok(html.includes("First.") && html.includes("Second."));
  assert.match(text, /First\.\n\nSecond\./);
  assert.match(text, /If this was you, there is nothing to do\./);
});

test("sent mail is multipart with the inline icon, reply-to and auto-reply headers", async () => {
  const send = createMailer({
    instanceName: "Commy",
    baseURL: "https://commy.chat",
    smtp: { transport: { streamTransport: true, buffer: true }, from: "Commy <no-reply@commy.chat>", replyTo: "help@commy.chat" },
  });
  const info = await send({ to: "a@example.test", subject: "Sign in", heading: "Sign in", body: "b", actionLabel: "Sign in", url: "https://commy.chat/x" });
  const raw = info.message.toString();
  assert.match(raw, /^Reply-To: help@commy\.chat/m);
  assert.match(raw, /^Auto-Submitted: auto-generated/m);
  assert.match(raw, /multipart\/alternative/);
  assert.match(raw, /Content-Id: <icon@commy>/i);
  assert.match(raw, /^Message-ID: <[^>]+@commy\.chat>/m);
});

test("turnstile needs both keys and its site key is public", () => {
  assert.equal(loadConfig({ ...base, TURNSTILE_SITE_KEY: "site" }).turnstile, null);
  const c = loadConfig({ ...base, TURNSTILE_SITE_KEY: "site", TURNSTILE_SECRET_KEY: "secret" });
  assert.deepEqual(c.turnstile, { siteKey: "site", secretKey: "secret" });
  const d = JSON.stringify(discoveryDocument(c));
  assert.ok(d.includes('"turnstileSiteKey":"site"'));
  assert.equal(d.includes("secret"), false);
});

test("mail without SMTP logs neither the recipient nor the link", async () => {
  const lines = [];
  const original = console.warn;
  console.warn = (...args) => lines.push(args.join(" "));
  try {
    await createMailer(loadConfig(base))({
      to: "victim@example.test",
      subject: "Reset",
      heading: "h",
      body: "b",
      actionLabel: "a",
      url: "https://x/reset?token=SECRET",
    });
  } finally {
    console.warn = original;
  }
  assert.equal(lines.length, 1);
  assert.equal(/victim|SECRET|https:/.test(lines[0]), false);
});

test("separate SMTP values build a STARTTLS transport on 587", () => {
  const c = loadConfig({
    ...base,
    SMTP_HOST: "smtp.mail.example",
    SMTP_USER: "no-reply@chat.example.org",
    SMTP_PASSWORD: "tok:en/with@chars",
  });
  assert.deepEqual(c.smtp.transport, {
    host: "smtp.mail.example",
    port: 587,
    secure: false,
    requireTLS: true,
    auth: { user: "no-reply@chat.example.org", pass: "tok:en/with@chars" },
  });
  assert.equal(c.smtp.from, "Commy <no-reply@chat.example.org>");
});

test("SMTP port 465 uses implicit TLS and EMAIL_FROM wins", () => {
  const c = loadConfig({ ...base, SMTP_HOST: "m", SMTP_PORT: "465", EMAIL_FROM: "Chat <hi@x.test>" });
  assert.equal(c.smtp.transport.secure, true);
  assert.equal(c.smtp.transport.requireTLS, false);
  assert.equal("auth" in c.smtp.transport, false);
  assert.equal(c.smtp.from, "Chat <hi@x.test>");
});

test("SMTP_URL takes its sender from the URL user", () => {
  const c = loadConfig({ ...base, SMTP_URL: "smtp://bot%40x.test:pw@m:587" });
  assert.equal(c.smtp.transport, "smtp://bot%40x.test:pw@m:587");
  assert.equal(c.smtp.from, "Commy <bot@x.test>");
});

test("SMTP misconfiguration is reported", () => {
  assert.throws(() => loadConfig({ ...base, SMTP_HOST: "m", SMTP_USER: "u" }), /SMTP_PASSWORD/);
  assert.throws(() => loadConfig({ ...base, SMTP_HOST: "m", SMTP_PORT: "0" }), /SMTP_PORT/);
  assert.throws(() => loadConfig({ ...base, SMTP_URL: "not a url" }), /SMTP_URL/);
});

test("twitch is enabled only with both values and appears in discovery", () => {
  assert.equal(loadConfig({ ...base, TWITCH_CLIENT_ID: "id" }).twitch, null);
  const c = loadConfig({ ...base, TWITCH_CLIENT_ID: "tid", TWITCH_CLIENT_SECRET: "tsec" });
  assert.deepEqual(c.twitch, { clientId: "tid", clientSecret: "tsec" });
  const d = discoveryDocument(c);
  assert.equal(d.providers.twitch, true);
  assert.equal(d.providers.google, false);
  assert.equal(JSON.stringify(d).includes("tsec"), false);
});

test("socialProviders lists configured providers and honours ALLOW_SIGNUP", () => {
  assert.deepEqual(socialProviders(loadConfig(base)), {});
  const c = loadConfig({
    ...base,
    GOOGLE_CLIENT_ID: "g",
    GOOGLE_CLIENT_SECRET: "gs",
    TWITCH_CLIENT_ID: "t",
    TWITCH_CLIENT_SECRET: "ts",
    ALLOW_SIGNUP: "false",
  });
  const p = socialProviders(c);
  assert.deepEqual(Object.keys(p), ["google", "twitch"]);
  assert.deepEqual(p.twitch, { clientId: "t", clientSecret: "ts", disableSignUp: true });
});
