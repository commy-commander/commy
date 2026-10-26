import assert from "node:assert/strict";
import test from "node:test";

import { MAX_UPLOAD_CHARS, UploadAssembler } from "../src/uploads.ts";

test("parts arriving in any order join into one upload", () => {
  const uploads = new UploadAssembler();
  assert.ok(uploads.start({ upload: "u1", server: "s1", name: "boom", emoji: "💥", parts: 3 }));
  assert.equal(uploads.add({ upload: "u1", index: 2, data: "C" }), null);
  assert.equal(uploads.add({ upload: "u1", index: 0, data: "A" }), null);
  assert.equal(uploads.add({ upload: "u1", index: 0, data: "X" }), null);
  assert.deepEqual(uploads.add({ upload: "u1", index: 1, data: "B" }), { server: "s1", name: "boom", emoji: "💥", data: "ABC" });
  assert.equal(uploads.add({ upload: "u1", index: 1, data: "B" }), null);
});

test("parts from an older upload are ignored once a new one starts", () => {
  const uploads = new UploadAssembler();
  uploads.start({ upload: "old", server: "s1", name: "a", parts: 1 });
  uploads.start({ upload: "new", server: "s1", name: "b", parts: 1 });
  assert.equal(uploads.add({ upload: "old", index: 0, data: "A" }), null);
  assert.deepEqual(uploads.add({ upload: "new", index: 0, data: "B" }), { server: "s1", name: "b", emoji: null, data: "B" });
});

test("bad starts are refused and oversized uploads are dropped", () => {
  const uploads = new UploadAssembler();
  assert.ok(!uploads.start({ upload: "u", server: "", name: "a", parts: 1 }));
  assert.ok(!uploads.start({ upload: "u", server: "s1", name: "a", parts: 0 }));
  assert.ok(!uploads.start({ upload: "u", server: "s1", name: "a", parts: 100000 }));
  uploads.start({ upload: "u", server: "s1", name: "a", parts: 2 });
  assert.throws(() => uploads.add({ upload: "u", index: 0, data: "x".repeat(MAX_UPLOAD_CHARS + 1) }), /over 1 MB/);
  assert.equal(uploads.add({ upload: "u", index: 1, data: "x" }), null);
});
