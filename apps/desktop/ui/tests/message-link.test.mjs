import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { messageLink, parseMessageLink, isBareMessageLink, forwardBody, forwardTargets } = require("../scripts/message-link.js");

const ids = {
  server: "11111111-2222-4333-8444-555555555555",
  channel: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
  message: "99999999-8888-4777-8666-555555555555",
};

test("a message link round trips and is found inside other text", () => {
  const link = messageLink(ids);
  assert.equal(link, `commy://m/${ids.server}/${ids.channel}/${ids.message}`);
  assert.deepEqual(parseMessageLink(`look at this ${link.toUpperCase()} please`), { ...ids, link: link.toUpperCase() });
  assert.equal(isBareMessageLink(`  ${link} `), true);
  assert.equal(isBareMessageLink(`see ${link}`), false);
  assert.equal(parseMessageLink("commy://m/not/a/link"), null);
  assert.equal(parseMessageLink("https://commy.chat/i/abcdefgh"), null);
});

test("a forwarded message quotes every line and names where it came from", () => {
  assert.equal(forwardBody({ content: "one\ntwo", author: "ferris", where: "#general" }), "**Forwarded** from ferris in #general\n> one\n> two");
  assert.equal(forwardBody({ content: "hi", author: "nova" }), "**Forwarded** from nova\n> hi");
});

test("forward targets list writable text channels, groups once and direct messages, filtered by the query", () => {
  const servers = [
    { id: "s1", name: "Hangout", channels: [{ id: "c1", name: "general", kind: "text" }, { id: "c2", name: "lobby", kind: "voice" }, { id: "c3", name: "news", kind: "announcement", can_send: false }] },
    { id: "g1", name: "ferris, nova", group: true, channels: [{ id: "g1c", name: "chat", kind: "text" }, { id: "g1d", name: "more", kind: "text" }] },
  ];
  const dms = [{ user: "u1", name: "ferris", kind: "inbox" }, { user: "u2", name: "spammer", kind: "spam" }];
  const all = forwardTargets({ servers, dms });
  assert.deepEqual(all.map(t => t.name), ["#general", "ferris, nova", "ferris"]);
  assert.deepEqual(all[0], { kind: "channel", server: "s1", channel: "c1", name: "#general", sub: "Hangout" });
  assert.deepEqual(all[1], { kind: "group", server: "g1", channel: "g1c", name: "ferris, nova", sub: "Group" });
  assert.deepEqual(forwardTargets({ servers, dms, query: "hang" }).map(t => t.name), ["#general"]);
  assert.deepEqual(forwardTargets({ servers, dms, query: "ferris" }).map(t => t.name), ["ferris, nova", "ferris"]);
});
