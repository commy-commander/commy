import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  SENSITIVE_PREVIEW, isSensitiveChannel, readConfirmed, writeConfirmed, sensitiveGated, showsSensitiveBadge,
  sensitivePreview, sensitiveNotice, sensitiveGateText, fallbackChannel,
} = require("../scripts/sensitive.js");

const adult = { id: "a", name: "after-dark", kind: "text", sensitive: true };
const plain = { id: "p", name: "general", kind: "text", sensitive: false };

test("only text-like channels with the flag count as sensitive", () => {
  assert.equal(isSensitiveChannel(adult), true);
  assert.equal(isSensitiveChannel({ ...adult, kind: "announcement" }), true);
  assert.equal(isSensitiveChannel({ ...adult, kind: "voice" }), false);
  assert.equal(isSensitiveChannel({ ...adult, kind: "category" }), false);
  assert.equal(isSensitiveChannel(plain), false);
  assert.equal(isSensitiveChannel(null), false);
  assert.equal(showsSensitiveBadge(adult), true);
  assert.equal(showsSensitiveBadge({ ...adult, kind: "voice" }), false);
});

test("confirmations survive a round trip and ignore junk", () => {
  assert.deepEqual([...readConfirmed(null)], []);
  assert.deepEqual([...readConfirmed("not json")], []);
  assert.deepEqual([...readConfirmed('{"a":1}')], []);
  assert.deepEqual([...readConfirmed('["a", 3, null, "b"]')], ["a", "b"]);
  const set = new Set(["x", "y"]);
  assert.deepEqual([...readConfirmed(writeConfirmed(set))], ["x", "y"]);
  assert.equal(writeConfirmed(null), "[]");
});

test("a sensitive channel is gated until confirmed on this device, and only in server view", () => {
  const none = new Set();
  assert.equal(sensitiveGated({ view: "server", channel: adult, confirmed: none }), true);
  assert.equal(sensitiveGated({ view: "server", channel: adult, confirmed: new Set(["a"]) }), false);
  assert.equal(sensitiveGated({ view: "server", channel: plain, confirmed: none }), false);
  assert.equal(sensitiveGated({ view: "dm", channel: adult, confirmed: none }), false);
  assert.equal(sensitiveGated({ view: "server", channel: null, confirmed: none }), false);
  assert.equal(sensitiveGated({ view: "server", channel: adult }), true);
});

test("previews from a gated channel say so instead of showing the text", () => {
  const none = new Set();
  assert.equal(sensitivePreview({ channel: adult, confirmed: none, text: "secret" }), SENSITIVE_PREVIEW);
  assert.equal(sensitivePreview({ channel: adult, confirmed: new Set(["a"]), text: "secret" }), "secret");
  assert.equal(sensitivePreview({ channel: plain, confirmed: none, text: "hello" }), "hello");
  assert.equal(sensitivePreview({ channel: null, confirmed: none, text: "hello" }), "hello");
});

test("notification payloads from a gated channel hide the message and its attachments", () => {
  const info = { sensitive: true, content: "secret", has_attachments: true, author_name: "nova" };
  const hidden = sensitiveNotice(info, "a", new Set());
  assert.equal(hidden.content, SENSITIVE_PREVIEW);
  assert.equal(hidden.has_attachments, false);
  assert.equal(hidden.author_name, "nova");
  assert.equal(sensitiveNotice(info, "a", new Set(["a"])), info);
  const open = { ...info, sensitive: false };
  assert.equal(sensitiveNotice(open, "a", new Set()), open);
  assert.equal(sensitiveNotice(null, "a", new Set()), null);
});

test("the gate names the channel", () => {
  const text = sensitiveGateText("after-dark");
  assert.equal(text.title, "This channel is age-restricted");
  assert.match(text.body, /#after-dark/);
  assert.match(text.body, /18 or older/);
});

test("nevermind goes back to the previous channel, or the first open one", () => {
  const none = new Set();
  const voice = { id: "v", name: "General", kind: "voice" };
  const locked = { id: "l", name: "mods", kind: "text", can_view: false };
  const channels = [voice, locked, adult, plain];
  assert.equal(fallbackChannel({ channels, previous: plain, confirmed: none }), plain);
  assert.equal(fallbackChannel({ channels, previous: adult, confirmed: none }), plain);
  assert.equal(fallbackChannel({ channels, previous: { id: "gone", kind: "text" }, confirmed: none }), plain);
  assert.equal(fallbackChannel({ channels, previous: null, confirmed: none }), plain);
  assert.equal(fallbackChannel({ channels: [voice, locked, adult], previous: null, confirmed: none }), null);
  assert.equal(fallbackChannel({ channels: [voice, adult], previous: null, confirmed: new Set(["a"]) }), adult);
});
