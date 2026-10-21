const MAX_FORUM_TAGS = 20;
const MAX_FORUM_TAG_CHARS = 20;
const MAX_POST_TAGS = 5;
const MAX_POST_TITLE_CHARS = 100;
const POST_EXCERPT_CHARS = 160;
const FORUM_MODES = ["list", "gallery"];

function isForumChannel(channel) {
  return Boolean(channel) && channel.kind === "forum";
}

function forumMode(stored) {
  return FORUM_MODES.includes(stored) ? stored : "list";
}

function tagLabel(tag) {
  if (!tag) return "";
  return tag.emoji ? `${tag.emoji} ${tag.name}` : tag.name;
}

function forumTagsById(tags) {
  return new Map((tags || []).map(t => [t.id, t]));
}

function filterPostsByTag(posts, tagId) {
  const list = posts || [];
  if (!tagId) return [...list];
  return list.filter(p => (p.tags || []).includes(tagId));
}

function sortPostsByActivity(posts) {
  return [...(posts || [])].sort((a, b) => (b.last_activity || 0) - (a.last_activity || 0) || (b.created_at || 0) - (a.created_at || 0));
}

function postExcerpt(content, max = POST_EXCERPT_CHARS) {
  const text = String(content || "")
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/<sticker:[^>]*>/g, "")
    .replace(/<a?:[a-z0-9_]+:[^>]*>/g, "")
    .replace(/<t:\d+(?::[a-zA-Z])?>/g, "")
    .replace(/\[([^\]]+)\]\((https?:[^)]+)\)/g, "$1")
    .replace(/\|\|/g, "")
    .replace(/(\*\*|__|~~|`|\*|_)/g, "")
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/^>\s?/gm, "")
    .replace(/^\s*(?:[-*+]|\d+\.)\s+/gm, "")
    .split(/\n+/)
    .map(line => line.replace(/\s+/g, " ").trim())
    .filter(Boolean)
    .join(" ");
  if (text.length <= max) return text;
  const cut = text.slice(0, max);
  const space = cut.lastIndexOf(" ");
  return (space > max * 0.6 ? cut.slice(0, space) : cut).trimEnd() + "\u2026";
}

function threadRootId(message, byId) {
  let current = message;
  const seen = new Set();
  while (current.reply_to && byId.has(current.reply_to) && !seen.has(current.id)) {
    seen.add(current.id);
    current = byId.get(current.reply_to);
  }
  return current.id;
}

function postStats(rootId, messages) {
  const list = messages || [];
  const byId = new Map(list.map(m => [m.id, m]));
  const root = byId.get(rootId);
  const created = root ? Date.parse(root.created_at) || 0 : 0;
  let replies = 0;
  let last = created;
  for (const m of list) {
    if (!m.reply_to || m.id === rootId) continue;
    if (threadRootId(m, byId) !== rootId) continue;
    replies += 1;
    last = Math.max(last, Date.parse(m.created_at) || 0);
  }
  return { replies, created_at: created, last_activity: last, root };
}

function forumPostViews(posts, messages, nameOf) {
  const views = [];
  for (const post of posts || []) {
    const stats = postStats(post.message, messages);
    if (!stats.root) continue;
    views.push({
      message: post.message,
      title: post.title,
      tags: [...(post.tags || [])],
      author: post.author,
      author_name: nameOf ? nameOf(post.author) : post.author,
      replies: stats.replies,
      created_at: stats.created_at,
      last_activity: stats.last_activity,
      excerpt: postExcerpt(stats.root.content, 240),
    });
  }
  return sortPostsByActivity(views);
}

function replyCountLabel(n) {
  const count = Number(n) || 0;
  if (count === 0) return "No replies";
  return count === 1 ? "1 reply" : `${count} replies`;
}

function activityLabel(at, now = Date.now()) {
  const ms = Number(at) || 0;
  if (!ms) return "";
  const seconds = Math.max(0, Math.round((now - ms) / 1000));
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  if (seconds < 7 * 86400) return `${Math.floor(seconds / 86400)}d ago`;
  return new Date(ms).toLocaleDateString([], { month: "short", day: "numeric" });
}

function togglePostTag(picked, tagId, max = MAX_POST_TAGS) {
  const list = picked || [];
  if (list.includes(tagId)) return list.filter(t => t !== tagId);
  if (list.length >= max) return [...list];
  return [...list, tagId];
}

function postDraftState({ title, body, tags }) {
  const cleanTitle = String(title || "").replace(/\s+/g, " ").trim();
  const cleanBody = String(body || "").trim();
  const picks = (tags || []).slice(0, MAX_POST_TAGS);
  return {
    title: cleanTitle,
    body: cleanBody,
    tags: picks,
    canSave: cleanTitle.length > 0 && cleanTitle.length <= MAX_POST_TITLE_CHARS && cleanBody.length > 0,
    titleLeft: MAX_POST_TITLE_CHARS - cleanTitle.length,
    tagsLeft: MAX_POST_TAGS - picks.length,
  };
}

function cleanTagName(name) {
  return String(name || "").replace(/\s+/g, " ").trim().slice(0, MAX_FORUM_TAG_CHARS);
}

function tagsAfterAdd(tags, name, emoji) {
  const list = tags || [];
  const clean = cleanTagName(name);
  if (!clean || list.length >= MAX_FORUM_TAGS) return null;
  if (list.some(t => t.name.toLowerCase() === clean.toLowerCase())) return null;
  return [...list, { id: null, name: clean, emoji: emoji || null }];
}

function tagsAfterRemove(tags, index) {
  return (tags || []).filter((_, i) => i !== index);
}

function canEditPost(post, me, canManageMessages) {
  if (!post || !me) return false;
  return post.author === me || Boolean(canManageMessages);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    MAX_FORUM_TAGS, MAX_FORUM_TAG_CHARS, MAX_POST_TAGS, MAX_POST_TITLE_CHARS, FORUM_MODES,
    isForumChannel, forumMode, tagLabel, forumTagsById, filterPostsByTag, sortPostsByActivity, postExcerpt,
    postStats, forumPostViews, replyCountLabel, activityLabel, togglePostTag, postDraftState, cleanTagName,
    tagsAfterAdd, tagsAfterRemove, canEditPost,
  };
}
