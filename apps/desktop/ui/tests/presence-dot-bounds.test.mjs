import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";

const stylesDir = new URL("../styles/", import.meta.url);
const sheets = readdirSync(stylesDir)
  .filter(name => name.endsWith(".css"))
  .map(name => ({ name, css: readFileSync(new URL(name, stylesDir), "utf8").replace(/\r\n/g, "\n").replace(/\/\*[\s\S]*?\*\//g, "") }));

const rulesOf = css => [...css.replace(/@media[^{]+\{/g, "").matchAll(/([^{}]+)\{([^{}]*)\}/g)]
  .map(([, selector, body]) => ({ selector: selector.trim(), body }));

const dotRules = sheets.flatMap(({ name, css }) => rulesOf(css)
  .flatMap(({ selector, body }) => selector.split(",").map(part => ({ name, selector: part.trim(), body })))
  .filter(({ selector }) => /\.presence-dot(?![\w-])[^\s>+~]*$/.test(selector) && !/::?(before|after)$/.test(selector)));

const positionOf = body => (body.match(/(?:^|[;\s])position:\s*([\w-]+)/) || [])[1];

test("every presence dot stays a positioned box so its phone mark cannot spread over the page", () => {
  const unpositioned = dotRules
    .filter(({ body }) => ["static", "unset", "initial", "revert"].includes(positionOf(body)))
    .map(({ name, selector }) => `${name}: ${selector}`);
  assert.deepEqual(unpositioned, []);
});

test("the inline dot next to a name sits in the line without drifting off it", () => {
  const inline = dotRules.find(({ name, selector }) => name === "styles.css" && selector === ".presence-dot.inline");
  assert.ok(inline);
  assert.equal(positionOf(inline.body), "relative");
  assert.match(inline.body, /right:\s*auto;/);
  assert.match(inline.body, /bottom:\s*auto;/);
});

test("the phone mark layers are pinned to the dot itself", () => {
  const styles = sheets.find(({ name }) => name === "styles.css").css;
  for (const layer of ["::before", "::after"]) {
    const rule = rulesOf(styles).find(({ selector }) => selector === `.presence-dot.phone${layer}`);
    assert.ok(rule, layer);
    assert.match(rule.body, /position:\s*absolute;/);
    assert.match(rule.body, /inset:\s*0;/);
  }
});
