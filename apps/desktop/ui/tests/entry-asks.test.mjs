import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { queueEntryAsk, entryAsksFromSameDevice, withoutEntryAsks, withoutEntryAskId } = require("../scripts/entry-asks.js");

const stale = { id: 1, device_name: "Web browser", code: "553 987" };
const fresh = { id: 2, device_name: "Web browser", code: "701 887" };
const sameDeviceOverP2P = { id: 3, device_name: "Web browser", code: "701 887" };

test("a new request is shown ahead of one that is still waiting, so a browser that left cannot block the next", () => {
  const queue = queueEntryAsk(queueEntryAsk([], stale), fresh);
  assert.deepEqual(queue.map(a => a.id), [2, 1]);
});

test("the same request arriving twice is kept once", () => {
  assert.deepEqual(queueEntryAsk([fresh, stale], fresh).map(a => a.id), [2, 1]);
});

test("answering a device answers every request it sent, and nothing from another device", () => {
  const queue = [sameDeviceOverP2P, fresh, stale];
  const answered = entryAsksFromSameDevice(queue, sameDeviceOverP2P);
  assert.deepEqual(answered.map(a => a.id), [3, 2]);
  assert.deepEqual(withoutEntryAsks(queue, answered).map(a => a.id), [1]);
});

test("a request without a code only answers itself", () => {
  const plain = { id: 4, device_name: "Laptop", code: null };
  const other = { id: 5, device_name: "Phone", code: null };
  assert.deepEqual(entryAsksFromSameDevice([plain, other], plain).map(a => a.id), [4]);
});

test("a request the desktop closed leaves the queue", () => {
  assert.deepEqual(withoutEntryAskId([fresh, stale], 1).map(a => a.id), [2]);
});
