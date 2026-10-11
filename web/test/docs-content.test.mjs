import assert from "node:assert/strict";
import { test } from "node:test";
import { excerpt, parseFrontmatter, plainText, searchDocs, slugify, sortDocs, toMeta } from "../lib/docs-content.ts";

test("frontmatter is parsed and stripped", () => {
  const { meta, body } = parseFrontmatter("---\r\ntitle: Hello: world\r\norder: 2\r\n---\r\n# Body\r\n");
  assert.deepEqual(meta, { title: "Hello: world", order: "2" });
  assert.equal(body, "# Body\n");
  assert.deepEqual(parseFrontmatter("no fences").meta, {});
});

test("metadata falls back sensibly", () => {
  assert.deepEqual(toMeta("faq", {}), { slug: "faq", title: "faq", nav: "faq", description: "", order: 999 });
  assert.equal(toMeta("x", { title: "T", nav: "N", order: "3" }).nav, "N");
  const sorted = sortDocs([toMeta("b", { title: "B", order: "2" }), toMeta("a", { title: "A", order: "2" }), toMeta("z", { order: "0" })]);
  assert.deepEqual(sorted.map((d) => d.slug), ["z", "a", "b"]);
});

test("slugify makes stable anchors", () => {
  assert.equal(slugify("1. Install the app"), "1-install-the-app");
  assert.equal(slugify("What it <em>never</em> sees &amp; why"), "what-it-never-sees-why");
});

test("plain text drops markup and code", () => {
  const md = "## Title\n\nSee [docs](/docs) and `code`.\n\n```bash\nrm -rf /\n```\n- **bold** item";
  const text = plainText(md);
  assert.ok(text.includes("See docs and code"));
  assert.ok(!text.includes("rm -rf"));
  assert.ok(!text.includes("/docs"));
  assert.ok(!text.includes("**"));
});

test("search needs every term and ranks title matches first", () => {
  const entries = [
    { slug: "voice", title: "Voice", description: "noise cancellation", text: "voice channels and noise" },
    { slug: "privacy", title: "Privacy", description: "what is stored", text: "voice and noise never stored" },
  ];
  assert.deepEqual(searchDocs(entries, "noise").map((e) => e.slug), ["voice", "privacy"]);
  assert.deepEqual(searchDocs(entries, "privacy voice").map((e) => e.slug), ["privacy"]);
  assert.deepEqual(searchDocs(entries, "   "), []);
  assert.deepEqual(searchDocs(entries, "nothinglikethis"), []);
});

test("excerpts center on the match", () => {
  const text = "a".repeat(200) + " relay " + "b".repeat(200);
  const e = excerpt(text, "relay", 10);
  assert.ok(e.startsWith("...") && e.endsWith("...") && e.includes("relay"));
  assert.equal(excerpt("short text", "zzz"), "short text");
});
