import assert from "node:assert/strict";
import { test } from "node:test";
import { createLoaders, isPagePath, providersFrom } from "../pages/src/loaders.js";
import { cleanToken, PLEDGE } from "../pages/src/lib/listing.js";

const APPLICATION = {
  origin: "https://chat.example.org",
  name: "Example Chat",
  about: null,
  kinds: ["home", "sync"],
  contact: "abuse@example.org",
  terms: "https://chat.example.org/terms",
  privacy: "https://chat.example.org/privacy",
  operatorEmail: "op@example.org",
  emergencyEmail: "backup@example.org",
  appliedAt: Date.parse("2026-11-15T12:00:00Z"),
  checks: [{ id: "tls", ok: true, detail: "The certificate is accepted." }],
};

function loaders(listing, answers = {}) {
  const asked = [];
  const load = createLoaders({
    fetchAuth: async (path) => {
      asked.push(path);
      return answers[path.split("?")[0]] ?? null;
    },
    name: "Commy",
    providers: providersFrom({}),
    hasAccess: () => true,
    listing,
  });
  return { load, asked };
}

test("the listing pages exist only where the listing is on", async () => {
  assert.ok(isPagePath("/servers/apply") && isPagePath("/servers/review"));
  const { load } = loaders(false);
  assert.equal(await load("/servers/apply", new URLSearchParams(), ""), null);
  assert.equal(await load("/servers/review", new URLSearchParams("token=abc.def"), ""), null);
});

test("the apply page carries only a clean confirmation token", async () => {
  const { load, asked } = loaders(true);
  const plain = await load("/servers/apply", new URLSearchParams(), "");
  assert.equal(plain.page, "list-server");
  assert.equal(plain.props.confirm, null);
  const confirming = await load("/servers/apply", new URLSearchParams("confirm=abc_DEF-123"), "");
  assert.equal(confirming.props.confirm, "abc_DEF-123");
  assert.equal(confirming.robots, "noindex");
  assert.equal((await load("/servers/apply", new URLSearchParams("confirm=<script>"), "")).props.confirm, null);
  assert.deepEqual(asked, []);
});

test("the review page reads the application its token names", async () => {
  const { load, asked } = loaders(true, { "/commy/listing/review": { application: APPLICATION } });
  const review = await load("/servers/review", new URLSearchParams("token=abc.def"), "");
  assert.equal(review.props.application.name, "Example Chat");
  assert.deepEqual(asked, ["/commy/listing/review?token=abc.def"]);
  const missing = await loaders(true).load("/servers/review", new URLSearchParams("token=abc.def"), "");
  assert.equal(missing.props.application, null);
});

test("both pages render on the server", async () => {
  const { renderPage } = await import("../pages/dist/server.js");
  const site = { name: "Commy", host: "commy.chat", logo: "/logo.svg", links: {} };
  const apply = renderPage({ page: "list-server", props: { confirm: null }, site });
  for (const [promise] of PLEDGE) assert.ok(apply.includes(promise.replace("'", "&#39;")) || apply.includes(promise), promise);
  assert.ok(renderPage({ page: "list-server", props: { confirm: "abc" }, site }).includes("Confirm the contact"));
  const review = renderPage({ page: "listing-review", props: { token: "t", application: APPLICATION }, site });
  assert.ok(review.includes("backup@example.org") && review.includes("Approve and list"));
  assert.ok(renderPage({ page: "listing-review", props: { token: "", application: null }, site }).includes("expired"));
});

test("tokens keep to their characters and length", () => {
  assert.equal(cleanToken("a.b_c-D"), "a.b_c-D");
  assert.equal(cleanToken("a b"), "");
  assert.equal(cleanToken("x".repeat(201), 200), "");
});
