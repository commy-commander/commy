import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  MAX_POST_TAGS, MAX_FORUM_TAGS, isForumChannel, forumMode, tagLabel, filterPostsByTag, sortPostsByActivity, postExcerpt,
  postStats, forumPostViews, replyCountLabel, activityLabel, togglePostTag, postDraftState, tagsAfterAdd, tagsAfterRemove, canEditPost,
} = require("../scripts/forum.js");

const at = minutesAgo => new Date(1_700_000_000_000 - minutesAgo * 60_000).toISOString();
const messages = [
  { id: "p1", content: "# Dark mode\nIt would be **great** to have it. See [the issue](https://commy.chat/x).", created_at: at(120), reply_to: null },
  { id: "r1", content: "yes please", created_at: at(90), reply_to: "p1" },
  { id: "r2", content: "me too", created_at: at(10), reply_to: "r1" },
  { id: "p2", content: "```rust\nfn main() {}\n```\nWhy does the borrow checker hate me?", created_at: at(60), reply_to: null },
  { id: "p3", content: "Lonely post", created_at: at(30), reply_to: null },
  { id: "x1", content: "a reply to a message that is not a post", created_at: at(5), reply_to: "missing" },
];
const posts = [
  { message: "p1", title: "Dark mode", tags: ["t1"], author: "m1" },
  { message: "p2", title: "Borrow checker", tags: ["t2", "t1"], author: "m2" },
  { message: "p3", title: "Lonely", tags: [], author: "me" },
  { message: "gone", title: "Root missing", tags: [], author: "me" },
];

test("only forum channels are forums and the view mode falls back to the list", () => {
  assert.equal(isForumChannel({ kind: "forum" }), true);
  assert.equal(isForumChannel({ kind: "text" }), false);
  assert.equal(isForumChannel(null), false);
  assert.equal(forumMode("gallery"), "gallery");
  assert.equal(forumMode("nonsense"), "list");
  assert.equal(forumMode(null), "list");
});

test("post views count replies down the thread, take the latest activity and drop posts without a root", () => {
  const views = forumPostViews(posts, messages, id => ({ m1: "ferris", m2: "nova" })[id] || "you");
  assert.deepEqual(views.map(v => v.message), ["p1", "p3", "p2"], "sorted by last activity, newest first");
  const dark = views[0];
  assert.equal(dark.replies, 2, "nested replies count towards the root");
  assert.equal(dark.last_activity, Date.parse(at(10)));
  assert.equal(dark.author_name, "ferris");
  assert.equal(views[2].replies, 0);
  assert.equal(views[2].last_activity, views[2].created_at);
  assert.equal(postStats("p3", messages).replies, 0);
});

test("filtering by tag keeps only posts carrying it and sorting is by activity", () => {
  assert.deepEqual(filterPostsByTag(posts, "t1").map(p => p.message), ["p1", "p2"]);
  assert.deepEqual(filterPostsByTag(posts, "t2").map(p => p.message), ["p2"]);
  assert.equal(filterPostsByTag(posts, null).length, 4);
  const sorted = sortPostsByActivity([{ message: "a", last_activity: 1 }, { message: "b", last_activity: 3 }, { message: "c", last_activity: 2 }]);
  assert.deepEqual(sorted.map(p => p.message), ["b", "c", "a"]);
});

test("the excerpt strips markdown, code and tokens and cuts on a word", () => {
  assert.equal(postExcerpt(messages[0].content), "Dark mode It would be great to have it. See the issue.");
  assert.equal(postExcerpt(messages[3].content), "Why does the borrow checker hate me?");
  assert.equal(postExcerpt("<:ferris:abc@node> hello <sticker:party:p@n> there <t:1760900000:R>"), "hello there");
  const long = postExcerpt("word ".repeat(80), 40);
  assert.ok(long.endsWith("…"));
  assert.ok(long.length <= 41);
  assert.ok(long.endsWith("word\u2026"), "cut lands on a word boundary");
  assert.equal(postExcerpt(""), "");
});

test("labels read naturally", () => {
  assert.equal(tagLabel({ name: "Bug", emoji: "\u{1F41B}" }), "\u{1F41B} Bug");
  assert.equal(tagLabel({ name: "Idea", emoji: null }), "Idea");
  assert.equal(replyCountLabel(0), "No replies");
  assert.equal(replyCountLabel(1), "1 reply");
  assert.equal(replyCountLabel(7), "7 replies");
  const now = 1_700_000_000_000;
  assert.equal(activityLabel(now - 20_000, now), "just now");
  assert.equal(activityLabel(now - 5 * 60_000, now), "5m ago");
  assert.equal(activityLabel(now - 3 * 3_600_000, now), "3h ago");
  assert.equal(activityLabel(now - 2 * 86_400_000, now), "2d ago");
  assert.equal(activityLabel(0, now), "");
});

test("tag picks toggle and stop at the cap", () => {
  assert.deepEqual(togglePostTag([], "a"), ["a"]);
  assert.deepEqual(togglePostTag(["a"], "a"), []);
  const full = ["1", "2", "3", "4", "5"];
  assert.equal(full.length, MAX_POST_TAGS);
  assert.deepEqual(togglePostTag(full, "6"), full);
  assert.deepEqual(togglePostTag(full, "3"), ["1", "2", "4", "5"]);
});

test("a draft saves only with a title and a body", () => {
  assert.equal(postDraftState({ title: "  Hi  there ", body: "x", tags: [] }).canSave, true);
  assert.equal(postDraftState({ title: "  Hi  there ", body: "x", tags: [] }).title, "Hi there");
  assert.equal(postDraftState({ title: "", body: "x", tags: [] }).canSave, false);
  assert.equal(postDraftState({ title: "t", body: "  ", tags: [] }).canSave, false);
  assert.equal(postDraftState({ title: "x".repeat(101), body: "b", tags: [] }).canSave, false);
  assert.equal(postDraftState({ title: "t", body: "b", tags: ["1", "2", "3", "4", "5", "6"] }).tags.length, MAX_POST_TAGS);
});

test("the tags editor refuses blanks, duplicates and more than the cap", () => {
  const tags = [{ id: "t1", name: "Bug", emoji: null }];
  assert.equal(tagsAfterAdd(tags, "  bug ", null), null);
  assert.equal(tagsAfterAdd(tags, "   ", null), null);
  const added = tagsAfterAdd(tags, "Feature   request", "\u{1F4A1}");
  assert.deepEqual(added[1], { id: null, name: "Feature request", emoji: "\u{1F4A1}" });
  const many = Array.from({ length: MAX_FORUM_TAGS }, (_, i) => ({ id: String(i), name: `t${i}`, emoji: null }));
  assert.equal(tagsAfterAdd(many, "one more", null), null);
  assert.deepEqual(tagsAfterRemove(added, 0).map(t => t.name), ["Feature request"]);
  assert.equal(tagsAfterAdd(tags, "x".repeat(40), null)[1].name.length, 20);
});

test("the author or a message manager may edit a post", () => {
  const post = { author: "m1" };
  assert.equal(canEditPost(post, "m1", false), true);
  assert.equal(canEditPost(post, "m2", false), false);
  assert.equal(canEditPost(post, "m2", true), true);
  assert.equal(canEditPost(null, "m1", true), false);
});
