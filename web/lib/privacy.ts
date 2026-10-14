import fs from "node:fs";
import path from "node:path";
import { policyHtml } from "./privacy-content";

// Single source for /privacy. Edit that file, never the rendered page.
export const POLICY_FILE = path.join(process.cwd(), "content", "privacy-policy.html");

export function getPrivacyPolicy(): string {
  return policyHtml(fs.readFileSync(POLICY_FILE, "utf8"));
}
