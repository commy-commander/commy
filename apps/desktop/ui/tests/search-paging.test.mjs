import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  SEARCH_PAGE_SIZE, searchScopes, defaultSearchScope, pickSearchScope, searchPlace, searchScopeTitle, searchScopeLine,
  searchRequest, clampSearchOffset, pageWindow, pageCount, pageLabel, dmHitKey, groupHits, searchHitMessage,
} = require("../scripts/search-paging.js");

const dmView = { view: "dm", group: false, conversation: true, conversationName: "ferris", dmUser: "u1", serverId: null, channelId: null };
const dmHome = { view: "dm", group: false, conversation: false, conversationName: "", dmUser: null, serverId: null, channelId: null };
const groupView = { view: "server", group: true, conversation: true, conversationName: "ferris, nova", dmUser: null, serverId: "g1", channelId: "gc1" };
const serverView = { view: "server", group: false, conversation: true, conversationName: "general", dmUser: null, serverId: "s1", channelId: "c1" };
const args = { terms: ["lunch"], from: null, mentions: null, channelId: null };

test("the scope control always has the same three slots, and only the ones that make sense are enabled", () => {
  const keys = scopes => scopes.map(s => s.key);
  const enabled = scopes => scopes.filter(s => s.enabled).map(s => s.key);
  assert.deepEqual(keys(searchScopes(dmView)), ["conversation", "dms", "server"]);
  assert.deepEqual(enabled(searchScopes(dmView)), ["conversation", "dms"]);
  assert.deepEqual(enabled(searchScopes(dmHome)), ["dms"]);
  assert.deepEqual(enabled(searchScopes(groupView)), ["conversation", "dms"]);
  assert.deepEqual(enabled(searchScopes(serverView)), ["conversation", "dms", "server"]);
  assert.deepEqual(enabled(searchScopes({ view: "server", group: false, conversation: false })), ["dms", "server"]);
});

test("a direct message or group searches that conversation by default, a server searches the whole server", () => {
  assert.equal(defaultSearchScope(dmView), "conversation");
  assert.equal(defaultSearchScope(groupView), "conversation");
  assert.equal(defaultSearchScope(dmHome), "dms");
  assert.equal(defaultSearchScope(serverView), "server");
});

test("a chosen scope survives while it still applies and falls back when it does not", () => {
  assert.equal(pickSearchScope("dms", dmView), "dms");
  assert.equal(pickSearchScope("conversation", dmView), "conversation");
  assert.equal(pickSearchScope("server", dmView), "conversation");
  assert.equal(pickSearchScope("conversation", dmHome), "dms");
  assert.equal(pickSearchScope(null, serverView), "server");
  assert.equal(pickSearchScope("conversation", serverView), "conversation");
});

test("the place key changes when the conversation changes, not when a channel inside a server does", () => {
  assert.equal(searchPlace(dmView), "dm::u1:1");
  assert.notEqual(searchPlace(dmView), searchPlace(dmHome));
  assert.notEqual(searchPlace(dmView), searchPlace({ ...dmView, dmUser: "u2" }));
  assert.equal(searchPlace(serverView), searchPlace({ ...serverView, channelId: "c2", conversationName: "news" }));
  assert.notEqual(searchPlace(serverView), searchPlace(groupView));
});

test("titles and sub lines name the scope", () => {
  assert.equal(searchScopeTitle("conversation", dmView), "Search this conversation");
  assert.equal(searchScopeTitle("conversation", serverView), "Search this channel");
  assert.equal(searchScopeTitle("dms", dmView), "Search your direct messages");
  assert.equal(searchScopeTitle("server", serverView), "Search this server");
  assert.equal(searchScopeLine("conversation", { terms: [], channel: null }, dmView), "With @ferris");
  assert.equal(searchScopeLine("conversation", { terms: [], channel: null }, groupView), "In ferris, nova");
  assert.equal(searchScopeLine("conversation", { terms: [], channel: null }, serverView), "In #general");
  assert.equal(searchScopeLine("server", { terms: [], channel: { id: "c2", name: "news" } }, serverView), "In #news");
  assert.equal(searchScopeLine("server", { terms: [], channel: null }, serverView), "Newest first, across this server");
  assert.equal(searchScopeLine("dms", { terms: [], channel: null }, serverView), "Newest first, across your direct messages");
});

test("the request picks the command and arguments for the scope", () => {
  assert.deepEqual(searchRequest("conversation", args, dmView, 0), {
    command: "search_dms",
    args: { terms: ["lunch"], from: null, mentions: null, userId: "u1", offset: 0 },
  });
  assert.deepEqual(searchRequest("dms", args, serverView, 100), {
    command: "search_dms",
    args: { terms: ["lunch"], from: null, mentions: null, userId: null, offset: 100 },
  });
  assert.deepEqual(searchRequest("conversation", args, serverView, 50), {
    command: "search_messages",
    args: { terms: ["lunch"], from: null, mentions: null, channelId: "c1", serverId: "s1", offset: 50 },
  });
  assert.equal(searchRequest("conversation", { ...args, channelId: "c9" }, serverView, 0).args.channelId, "c9");
  assert.deepEqual(searchRequest("conversation", args, groupView, 0).args, { terms: ["lunch"], from: null, mentions: null, channelId: null, serverId: "g1", offset: 0 });
  assert.deepEqual(searchRequest("server", args, serverView, -3).args, { terms: ["lunch"], from: null, mentions: null, channelId: null, serverId: "s1", offset: 0 });
});

test("offsets snap to page starts and never pass the last page", () => {
  assert.equal(SEARCH_PAGE_SIZE, 50);
  assert.equal(clampSearchOffset(0, 50), 0);
  assert.equal(clampSearchOffset(312, 100), 100);
  assert.equal(clampSearchOffset(312, 900), 300);
  assert.equal(clampSearchOffset(50, 50), 0);
  assert.equal(clampSearchOffset(51, 50), 50);
  assert.equal(clampSearchOffset(10, -5), 0);
});

test("a page window knows what it shows and where older and newer pages start", () => {
  assert.deepEqual(pageWindow(312, 0), { total: 312, offset: 0, size: 50, first: 1, last: 50, hasOlder: true, hasNewer: false, olderOffset: 50, newerOffset: 0 });
  assert.deepEqual(pageWindow(312, 50), { total: 312, offset: 50, size: 50, first: 51, last: 100, hasOlder: true, hasNewer: true, olderOffset: 100, newerOffset: 0 });
  assert.deepEqual(pageWindow(312, 300), { total: 312, offset: 300, size: 50, first: 301, last: 312, hasOlder: false, hasNewer: true, olderOffset: 300, newerOffset: 250 });
  assert.deepEqual(pageWindow(0, 0), { total: 0, offset: 0, size: 50, first: 0, last: 0, hasOlder: false, hasNewer: false, olderOffset: 0, newerOffset: 0 });
  assert.deepEqual(pageWindow(7, 50), { total: 7, offset: 0, size: 50, first: 1, last: 7, hasOlder: false, hasNewer: false, olderOffset: 0, newerOffset: 0 });
});

test("the page label counts small result sets and shows the window for big ones", () => {
  assert.equal(pageCount(0), "No messages found");
  assert.equal(pageCount(1), "1 result");
  assert.equal(pageCount(312), "312 results");
  assert.equal(pageLabel(0, 0), "No messages found");
  assert.equal(pageLabel(1, 0), "1 result");
  assert.equal(pageLabel(50, 0), "50 results");
  assert.equal(pageLabel(312, 0), "Showing 1 to 50 of 312");
  assert.equal(pageLabel(312, 300), "Showing 301 to 312 of 312");
  assert.equal(pageLabel(312, 999), "Showing 301 to 312 of 312");
});

test("hits group into consecutive runs per conversation, keeping newest first order", () => {
  const hits = [
    { kind: "dm", user: "u1", name: "ferris", message: { id: "a" } },
    { kind: "dm", user: "u1", name: "ferris", message: { id: "b" } },
    { kind: "group", server: "g1", name: "ferris, nova", message: { id: "c" } },
    { kind: "dm", user: "u1", name: "ferris", message: { id: "d" } },
  ];
  const runs = groupHits(hits);
  assert.deepEqual(runs.map(r => [r.key, r.label, r.hits.map(h => h.message.id)]), [
    ["dm:u1", "@ferris", ["a", "b"]],
    ["group:g1", "ferris, nova", ["c"]],
    ["dm:u1", "@ferris", ["d"]],
  ]);
  assert.deepEqual(groupHits([]), []);
  assert.deepEqual(dmHitKey({ kind: "dm", user: "u2" }), { key: "dm:u2", label: "@Unknown User" });
});

test("a direct message hit renders like a channel message", () => {
  const hit = { kind: "dm", user: "u1", name: "ferris", message: { id: "d1", from: "u1", from_name: "ferris", content: "hi", created_at: "2026-01-01T00:00:00Z" } };
  assert.deepEqual(searchHitMessage(hit), { id: "d1", author_id: "u1", author_name: "ferris", content: "hi", created_at: "2026-01-01T00:00:00Z", attachments: [] });
  const group = { kind: "group", server: "g1", name: "crew", channel: "c", message: { id: "g", author_id: "u2", author_name: "nova", content: "yo" } };
  assert.equal(searchHitMessage(group), group.message);
});
