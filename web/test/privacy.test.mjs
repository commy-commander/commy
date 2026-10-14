import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { linkTargets, policyHtml, visibleText } from "../lib/privacy-content.ts";

const FILE = path.join(import.meta.dirname, "..", "content", "privacy-policy.html");
const source = fs.readFileSync(FILE, "utf8");

// The policy text is the owner's. Change this hash only together with a new
// policy file supplied by the owner, never to make an edit pass.
const POLICY_SHA256 = "d75083ebf42b5847dcfac7d01d0ce3a5b35d9052c722649de17dd81a40d09c49";

test("policy file is the approved text", () => {
  const digest = crypto.createHash("sha256").update(source.replace(/\r\n?/g, "\n")).digest("hex");
  assert.equal(digest, POLICY_SHA256);
});

test("rendering keeps every word of the policy", () => {
  const rendered = policyHtml(source);
  assert.equal(visibleText(rendered), visibleText(source));
  assert.ok(visibleText(rendered).startsWith("PRIVACY POLICY Last updated"));
});

test("rendering keeps every link and section anchor", () => {
  const rendered = policyHtml(source);
  assert.deepEqual(linkTargets(rendered), linkTargets(source));
  for (const id of ["toc", "cookies", "contact", "request"]) assert.match(rendered, new RegExp(` id="${id}"`));
});

test("rendering drops inline styles and scripts only", () => {
  const sample = '<style>p{}</style><p style="color: red" data-custom-class="body_text" onclick="x()">Hi <b>there</b></p><script>alert(1)</script>';
  assert.equal(policyHtml(sample), '<p data-custom-class="body_text">Hi <b>there</b></p>');
  const rendered = policyHtml(source);
  assert.doesNotMatch(rendered, /<style|<script|\sstyle=|\son[a-z]+=/i);
});

test("visible text decodes entities and ignores markup", () => {
  assert.equal(visibleText("<style>x</style><div>A&nbsp;&amp;\n <span>B</span>&#39;s</div>"), "A & B 's");
});
