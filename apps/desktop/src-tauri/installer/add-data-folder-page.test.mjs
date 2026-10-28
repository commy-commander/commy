import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { addDataFolderPage } from "./add-data-folder-page.mjs";

const template = [
  "{{#if installer_hooks}}",
  '!include "{{installer_hooks}}"',
  "{{/if}}",
  "!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive",
  "!insertmacro MUI_PAGE_DIRECTORY",
  "",
  "Var AppStartMenuFolder",
].join("\r\n");

test("the data folder page follows the install location page", () => {
  const patched = addDataFolderPage(template, "C:\\build\\data-folder.nsh").split("\n");
  const directory = patched.indexOf("!insertmacro MUI_PAGE_DIRECTORY");
  assert.equal(patched[directory + 1], '!include "C:\\build\\data-folder.nsh"');
  assert.equal(patched.length, 8);
});

test("a template that moved on stops the build instead of shipping without the page", () => {
  assert.throws(() => addDataFolderPage(template.replace("MUI_PAGE_DIRECTORY", "MUI_PAGE_ELSEWHERE"), "x"), /found 0/);
  assert.throws(() => addDataFolderPage(`${template}\n!insertmacro MUI_PAGE_DIRECTORY`, "x"), /found 2/);
});

test("the page only uses variables the installer template declares before it", () => {
  const page = readFileSync(new URL("./data-folder.nsh", import.meta.url), "utf8");
  const used = new Set([...page.matchAll(/\$([A-Z][A-Za-z]+)/g)].map(m => m[1]));
  const declaredHere = new Set([...page.matchAll(/^Var (\w+)/gm)].map(m => m[1]));
  const builtIn = new Set(["APPDATA"]);
  const fromTemplate = [...used].filter(name => !declaredHere.has(name) && !builtIn.has(name)).sort();
  assert.deepEqual(fromTemplate, ["DeleteAppDataCheckboxState", "PassiveMode", "UpdateMode"]);
});
