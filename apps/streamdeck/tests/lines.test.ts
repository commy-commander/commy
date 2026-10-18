import assert from "node:assert/strict";
import test from "node:test";

import { LineBuffer, MAX_LINE_CHARS } from "../src/lines.ts";

test("lines split across chunks come out whole", () => {
  const buffer = new LineBuffer();
  assert.deepEqual(buffer.push('{"type":"wai'), []);
  assert.deepEqual(buffer.push('ting"}\n{"type":"ready","version":1}\r\n{"ty'), ['{"type":"waiting"}', '{"type":"ready","version":1}']);
  assert.deepEqual(buffer.push('pe":"played","id":1}\n'), ['{"type":"played","id":1}']);
});

test("blank lines are skipped", () => {
  assert.deepEqual(new LineBuffer().push("\n\n{}\n"), ["{}"]);
});

test("a line that never ends is refused", () => {
  const buffer = new LineBuffer();
  assert.throws(() => buffer.push("x".repeat(MAX_LINE_CHARS + 1)));
  assert.deepEqual(buffer.push("ok\n"), ["ok"]);
});
