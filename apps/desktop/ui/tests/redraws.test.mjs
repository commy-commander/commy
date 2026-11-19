import "./i18n-global.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { coalescedRuns, memberRowsShown, MEMBER_ROWS_STEP } = require("../scripts/redraws.js");

function gate() {
  let open;
  const opened = new Promise(resolve => { open = resolve; });
  return { opened, open };
}

test("a burst of updates while one redraw runs costs one more redraw, not one each", async () => {
  const seen = [];
  const first = gate();
  const redraw = coalescedRuns(async key => {
    seen.push(key);
    if (seen.length === 1) await first.opened;
  });
  const done = redraw("server");
  for (let i = 0; i < 20; i++) redraw("server");
  first.open();
  await done;
  assert.deepEqual(seen, ["server", "server"]);
});

test("a lone update redraws once", async () => {
  let runs = 0;
  const redraw = coalescedRuns(() => { runs += 1; });
  await redraw("server");
  assert.equal(runs, 1);
  await redraw("server");
  assert.equal(runs, 2);
});

test("different servers redraw on their own", async () => {
  const seen = [];
  const hold = gate();
  const redraw = coalescedRuns(async key => {
    seen.push(key);
    if (key === "a") await hold.opened;
  });
  const a = redraw("a");
  await redraw("b");
  assert.deepEqual(seen, ["a", "b"]);
  hold.open();
  await a;
});

test("a redraw that fails does not stop the next one", async () => {
  let runs = 0;
  const warn = console.warn;
  console.warn = () => {};
  try {
    const redraw = coalescedRuns(async () => {
      runs += 1;
      if (runs === 1) throw new Error("boom");
    });
    await redraw("server");
    await redraw("server");
    assert.equal(runs, 2);
  } finally {
    console.warn = warn;
  }
});

test("a big member group draws a page of rows and offers the rest", () => {
  assert.deepEqual(memberRowsShown(12, 0), { rows: 12, more: 0, next: MEMBER_ROWS_STEP * 2 });
  assert.deepEqual(memberRowsShown(3000, 0), { rows: MEMBER_ROWS_STEP, more: 3000 - MEMBER_ROWS_STEP, next: MEMBER_ROWS_STEP * 2 });
  const after = memberRowsShown(3000, MEMBER_ROWS_STEP * 2);
  assert.equal(after.rows, MEMBER_ROWS_STEP * 2);
  assert.equal(after.next, MEMBER_ROWS_STEP * 3);
  assert.deepEqual(memberRowsShown(150, 5000), { rows: 150, more: 0, next: 5000 + MEMBER_ROWS_STEP });
});
