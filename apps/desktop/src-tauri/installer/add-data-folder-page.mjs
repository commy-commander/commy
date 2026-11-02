import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const DIRECTORY_PAGE = "!insertmacro MUI_PAGE_DIRECTORY";
const OUR_HOOKS = /\{\{#if installer_hooks\}\}/;

export function addDataFolderPage(template, pagePath) {
  const lines = template.split(/\r?\n/);
  const at = lines.flatMap((line, index) => (line.trim() === DIRECTORY_PAGE ? [index] : []));
  if (at.length !== 1) throw new Error(`expected one "${DIRECTORY_PAGE}" line in the installer template, found ${at.length}`);
  if (!OUR_HOOKS.test(template)) throw new Error("the installer template no longer offers installer hooks");
  lines.splice(at[0] + 1, 0, `!include "${pagePath}"`);
  return lines.join("\n");
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [upstream, output] = process.argv.slice(2);
  if (!upstream || !output) throw new Error("usage: add-data-folder-page.mjs <upstream installer.nsi> <output installer.nsi>");
  const page = fileURLToPath(new URL("./data-folder.nsh", import.meta.url));
  writeFileSync(output, addDataFolderPage(readFileSync(upstream, "utf8"), page));
}
