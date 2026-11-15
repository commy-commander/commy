import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const { REPORT_MAX_PICKS, reportCandidates, reportPicked, reportDestination, reportSentToast } = require("../scripts/report-picks.js");

const repo = join(import.meta.dirname, "..", "..", "..", "..");
const shared = readFileSync(join(repo, "crates", "commy-sync", "src", "report.rs"), "utf8");
const html = readFileSync(join(import.meta.dirname, "..", "index.html"), "utf8");
const app = readFileSync(join(import.meta.dirname, "..", "scripts", "app.js"), "utf8");

const said = (id, author, content = `message ${id}`) => ({ id, author_id: author, content });

test("the app picks no more messages than a sync server takes", () => {
  assert.match(shared, new RegExp(`MAX_PICKED_MESSAGES: usize = ${REPORT_MAX_PICKS};`));
});

test("only the reported person's recent messages are offered, the clicked one ticked", () => {
  const messages = [said("a", "them"), said("b", "me"), said("c", "them"), said("d", "them", "")];
  const offered = reportCandidates(messages, "them", "c");
  assert.deepEqual(offered.map(c => [c.id, c.checked]), [["a", false], ["c", true]]);
  assert.deepEqual(reportPicked(offered), ["c"]);
  assert.deepEqual(reportCandidates(messages, "them", null).filter(c => c.checked), []);
  assert.deepEqual(reportCandidates(undefined, "them", "c"), []);
});

test("an older clicked message stays on offer when they wrote a lot since", () => {
  const messages = Array.from({ length: 30 }, (_, i) => said(`m${i}`, "them"));
  const offered = reportCandidates(messages, "them", "m2");
  assert.equal(offered.length, REPORT_MAX_PICKS);
  assert.equal(offered[0].id, "m2");
  assert.ok(offered[0].checked);
  offered.forEach(c => (c.checked = true));
  assert.equal(reportPicked(offered).length, REPORT_MAX_PICKS);
});

test("the dialog says where the report goes", () => {
  assert.match(reportDestination({ sync_host: "sync.example.org" }, true), /server owner and to the people who run sync\.example\.org/);
  assert.match(reportDestination({}, false), /^Your report goes privately to the people who run Commy\.chat/);
});

test("the confirmation names who got it and how to reach them", () => {
  assert.equal(reportSentToast({ owner: true, host: { operator: "Example Chat", contact: "abuse@example.org" } }), "Report sent to the server owner and Example Chat. Contact them at abuse@example.org.");
  assert.equal(reportSentToast({ owner: false, host: { operator: "Example Chat" } }), "Report sent to Example Chat.");
  assert.match(reportSentToast({ owner: true, host: null, host_problem: "x" }), /could not take it/);
});

test("the report dialog sends the picked messages and the script loads before the app", () => {
  assert.ok(html.includes('id="report-pick-list"'));
  assert.ok(html.indexOf('src="scripts/report-picks.js"') < html.indexOf('src="scripts/app.js"'));
  assert.match(app, /const messageIds = reportPicked\(reportChoices\)/);
});
