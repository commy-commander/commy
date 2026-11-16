import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const { HELP_PAGES, helpUrl, shownHost, movePlan, movedInWelcome, inviteHelpPage, syncHostGuideSteps, syncHostGuideDone, finishSyncHostGuide } = require("../scripts/move-guide.js");

const memoryStorage = () => {
  const items = new Map();
  return { getItem: k => (items.has(k) ? items.get(k) : null), setItem: (k, v) => items.set(k, String(v)), removeItem: k => items.delete(k) };
};

test("help links open the matching guide on the website", () => {
  assert.equal(helpUrl("moving"), "https://commy.chat/docs/moving-your-account");
  assert.equal(helpUrl("syncServers"), "https://commy.chat/docs/sync-servers");
  assert.equal(helpUrl("accountServers"), "https://commy.chat/docs/account-servers");
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

test("an invite's help explains the sync server only when it asks for trust", () => {
  assert.equal(inviteHelpPage({ community_host: { ask: true } }), "syncServers");
  assert.equal(inviteHelpPage({ community_host: { ask: false } }), "accountServers");
  assert.equal(inviteHelpPage(null), "accountServers");
});

test("the owner walkthrough is shown until it is dismissed, and can come back", () => {
  const storage = memoryStorage();
  assert.equal(syncHostGuideDone(storage), false);
  finishSyncHostGuide(storage, true);
  assert.equal(syncHostGuideDone(storage), true);
  finishSyncHostGuide(storage, false);
  assert.equal(syncHostGuideDone(storage), false);
  assert.equal(syncHostGuideDone({ getItem() { throw new Error("blocked"); } }), false);
  assert.equal(syncHostGuideSteps().length, 4);
});
