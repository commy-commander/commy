// Run after `next build`: the prerendered /privacy page must contain the
// policy text exactly as written in content/privacy-policy.html.
import fs from "node:fs";
import path from "node:path";
import { visibleText } from "../lib/privacy-content.ts";

const root = path.join(import.meta.dirname, "..");
const source = fs.readFileSync(path.join(root, "content", "privacy-policy.html"), "utf8");
const page = fs.readFileSync(path.join(root, ".next", "server", "app", "privacy.html"), "utf8");

const want = visibleText(source);
const got = visibleText(page);
if (want && got.includes(want)) {
  console.log(`/privacy matches content/privacy-policy.html (${want.length} characters)`);
} else {
  // Longest prefix of the policy that the page still contains.
  let lo = 0;
  let hi = want.length;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (got.includes(want.slice(0, mid))) lo = mid;
    else hi = mid - 1;
  }
  console.error(`/privacy differs from content/privacy-policy.html after character ${lo}:`);
  console.error(`  source: ${JSON.stringify(want.slice(Math.max(0, lo - 30), lo + 50))}`);
  process.exit(1);
}
