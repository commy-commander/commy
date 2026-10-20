import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";

const STYLES = new URL("../styles/", import.meta.url);
const GROUPING_AT_RULES = /^@(media|supports|container|layer|keyframes|-webkit-keyframes|document|scope|starting-style)\b/;

function blank(text) {
  return text.replace(/[^\n]/g, " ");
}

function withoutCommentsAndStrings(css) {
  return css.replace(/\/\*[\s\S]*?\*\/|"(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'/g, blank);
}

function blockProblems(css) {
  const source = withoutCommentsAndStrings(css);
  const stack = [];
  const problems = [];
  let preludeStart = 0;
  let line = 1;
  for (let i = 0; i < source.length; i++) {
    const ch = source[i];
    if (ch === "\n") line++;
    if (ch === "{") {
      const prelude = source.slice(preludeStart, i).trim();
      const parent = stack[stack.length - 1];
      if (parent && !parent.groups) problems.push(`line ${line}: "${prelude.slice(0, 60)}" opens inside "${parent.prelude.slice(0, 60)}" from line ${parent.line}`);
      stack.push({ prelude, line, groups: GROUPING_AT_RULES.test(prelude) });
      preludeStart = i + 1;
    } else if (ch === "}") {
      if (!stack.length) problems.push(`line ${line}: a closing brace with nothing open`);
      stack.pop();
      preludeStart = i + 1;
    } else if (ch === ";") {
      preludeStart = i + 1;
    }
  }
  for (const open of stack) problems.push(`line ${open.line}: "${open.prelude.slice(0, 60)}" is never closed`);
  return problems;
}

test("every stylesheet rule is closed before the next one starts", () => {
  const files = readdirSync(STYLES).filter(name => name.endsWith(".css"));
  assert.ok(files.length > 0);
  for (const name of files) {
    assert.deepEqual(blockProblems(readFileSync(new URL(name, STYLES), "utf8")), [], name);
  }
});

test("the brace check catches a rule pasted inside another", () => {
  assert.deepEqual(blockProblems(".a { color: red; }\n@media (min-width: 1px) { .b { color: red; } }"), []);
  assert.equal(blockProblems(".a {\n.b { color: red; }\n}").length, 1);
  assert.equal(blockProblems(".a { color: red;").length, 1);
});
