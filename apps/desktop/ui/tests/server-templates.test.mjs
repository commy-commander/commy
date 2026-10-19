import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { looksLikeTemplateCode, templateGroups, templateSummary } = require("../scripts/server-templates.js");

const gaming = {
  channels: [
    { name: "Info", kind: "category", category: null },
    { name: "announcements", kind: "announcement", category: 0 },
    { name: "general", kind: "text", category: 2 },
    { name: "Voice", kind: "category", category: null },
    { name: "Lobby", kind: "voice", category: 3 },
    { name: "Squad 1", kind: "voice", category: 3 },
    { name: "Empty", kind: "category", category: null },
  ],
  roles: [{ name: "Squad Lead", color: "5b8cff" }],
  has_rules: false,
};

test("a template code is recognised by its prefix, even with leading space", () => {
  assert.equal(looksLikeTemplateCode("  commy-template:abc"), true);
  assert.equal(looksLikeTemplateCode("commy://i/abc"), false);
  assert.equal(looksLikeTemplateCode(null), false);
});

test("the preview groups channels like the channel list: loose text, loose voice, then categories, dropping empty ones", () => {
  assert.deepEqual(templateGroups(gaming), [
    { category: "Text Channels", channels: [{ name: "general", kind: "text" }] },
    { category: "Info", channels: [{ name: "announcements", kind: "announcement" }] },
    { category: "Voice", channels: [{ name: "Lobby", kind: "voice" }, { name: "Squad 1", kind: "voice" }] },
  ]);
});

test("the summary counts text and voice channels, roles and rules", () => {
  assert.equal(templateSummary(gaming), "2 text channels, 2 voice channels, 1 role");
  assert.equal(templateSummary({ channels: [{ name: "general", kind: "text" }], roles: [], has_rules: true }), "1 text channel, 0 voice channels, rules");
});
