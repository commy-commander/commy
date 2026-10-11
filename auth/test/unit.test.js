import assert from "node:assert/strict";
import { test } from "node:test";
import { discoveryDocument, loadConfig } from "../src/config.js";
import { normalizeDeviceName, normalizeNodeId, toPublicDevice } from "../src/devices.js";
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
  assert.equal(c.smtp.url, "smtp://mail:25");
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
  assert.deepEqual(d.providers, { google: false, magicLink: true, emailPassword: true });
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

test("normalizeDeviceName strips control characters and caps length", () => {
  assert.equal(normalizeDeviceName("  Gaming\u0007 PC\n"), "Gaming PC");
  assert.equal(normalizeDeviceName(""), "Commy desktop");
  assert.equal(normalizeDeviceName("a".repeat(100)).length, 64);
});

test("toPublicDevice drops the owning user id", () => {
  const out = toPublicDevice({ id: "1", nodeId: "n", name: "x", userId: "secret", createdAt: 1, lastSeenAt: 2 });
  assert.equal("userId" in out, false);
});

test("emails escape user controlled values", () => {
  assert.equal(escapeHtml(`<a href="x">'&`), "&lt;a href=&quot;x&quot;&gt;&#39;&amp;");
  const { html, text } = renderEmail({ instanceName: "<b>", heading: "Hi", body: "b", actionLabel: "Go", url: "https://x/?a=1&b=2" });
  assert.ok(html.includes("&lt;b&gt;"));
  assert.ok(html.includes("a=1&amp;b=2"));
  assert.ok(text.includes("https://x/?a=1&b=2"));
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
