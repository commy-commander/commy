import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const { HELP_PAGES, helpUrl, shownHost, movePlan, movedInWelcome, signInHelp } = require("../scripts/move-guide.js");

test("help links open the matching guide on the website", () => {
  assert.equal(helpUrl("moving"), "https://commy.chat/docs/moving-your-account");
  assert.equal(helpUrl("syncServers"), "https://commy.chat/docs/sync-servers");
  assert.equal(helpUrl("accountServers"), "https://commy.chat/docs/account-servers");
  assert.equal(helpUrl("choosing"), "https://commy.chat/docs/choosing-a-server");
  assert.equal(helpUrl("nothing"), "https://commy.chat/docs");
});

test("every help page the app links to is a guide on the website", () => {
  const html = readFileSync(join(import.meta.dirname, "..", "index.html"), "utf8");
  const linked = new Set([...html.matchAll(/data-help="([^"]+)"/g)].map(m => m[1]));
  for (const page of linked) assert.ok(HELP_PAGES[page], `${page} has a guide`);
});

test("hosts are shown without the scheme or a trailing slash", () => {
  assert.equal(shownHost("https://friends.example/"), "friends.example");
  assert.equal(shownHost("friends.example"), "friends.example");
  assert.equal(shownHost(null), "");
});

test("the move plan names the new account server in every part", () => {
  const plan = movePlan("https://friends.example");
  assert.equal(plan.title, "Move your account to friends.example?");
  assert.equal(plan.steps.length, 3);
  assert.ok(plan.steps[0].includes("friends.example") && plan.others.includes("Signed in through friends.example") && plan.note.includes("friends.example"));
  assert.ok(plan.note.includes("a year") && plan.note.includes("delete your account"));
});

test("the welcome after moving names both account servers", () => {
  const welcome = movedInWelcome({ from: "https://commy.chat", to: "https://friends.example" });
  assert.equal(welcome.title, "Welcome to friends.example");
  assert.ok(welcome.lead.includes("from commy.chat"));
  assert.ok(welcome.points.some(p => p.startsWith("commy.chat keeps a note")));
});

test("sign in points to the trust guide only when the server is not on Commy's list", () => {
  assert.deepEqual(signInHelp("signin.server.warning"), { page: "choosing", key: "signin.server.trust" });
  assert.deepEqual(signInHelp("signin.server.listed"), { page: "accountServers", key: "signin.server.help" });
  assert.deepEqual(signInHelp("signin.server.note"), { page: "accountServers", key: "signin.server.help" });
});

test("invite warnings link to the guide on choosing a server", () => {
  const html = readFileSync(join(import.meta.dirname, "..", "index.html"), "utf8");
  const phone = readFileSync(join(import.meta.dirname, "..", "scripts", "studio-phone.js"), "utf8");
  assert.match(html, /id="invite-link-learn" data-help="choosing"/);
  assert.match(phone, /id="pt-invite-learn" data-help="choosing"/);
});

