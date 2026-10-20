const THREAD_NAME_WORDS = 6;
const THREAD_NAME_CHARS = 48;
const MAX_THREAD_NAME_CHARS = 100;

function threadRootOf(message, byId) {
  let current = message;
  const seen = new Set();
  while (current.reply_to && byId.has(current.reply_to) && !seen.has(current.id)) {
    seen.add(current.id);
    current = byId.get(current.reply_to);
  }
  return current.id;
}

function threadReplyCounts(messages) {
  const byId = new Map(messages.map(m => [m.id, m]));
  const counts = new Map();
  for (const m of messages) {
    if (!m.reply_to) continue;
    const root = threadRootOf(m, byId);
    if (root !== m.id) counts.set(root, (counts.get(root) || 0) + 1);
  }
  return counts;
}

function defaultThreadName(text, fallback = "Thread") {
  const words = String(text || "").replace(/\s+/g, " ").trim();
  if (!words) return fallback;
  const picked = words.split(" ").slice(0, THREAD_NAME_WORDS).join(" ");
  const clipped = picked.length > THREAD_NAME_CHARS ? picked.slice(0, THREAD_NAME_CHARS - 1).trimEnd() : picked;
  return clipped.length < words.length ? `${clipped}…` : clipped;
}

function threadName(root, meta, plain = s => s) {
  if (meta && meta.name) return meta.name;
  if (!root) return "Thread";
  const fallback = (root.attachments || []).length ? "Attachment" : "Thread";
  return defaultThreadName(plain(root.content || ""), fallback);
}

function cleanThreadName(name) {
  const clean = String(name || "").replace(/\s+/g, " ").trim();
  if (!clean) return { name: null };
  if ([...clean].length > MAX_THREAD_NAME_CHARS) return { error: `Thread names can be at most ${MAX_THREAD_NAME_CHARS} characters` };
  return { name: clean };
}

function buildThreadList(messages, threads, { plain = s => s } = {}) {
  const list = messages || [];
  const byId = new Map(list.map(m => [m.id, m]));
  const metaByRoot = new Map((threads || []).map(t => [t.root, t]));
  const stats = new Map();
  for (const m of list) {
    if (!m.reply_to) continue;
    const root = threadRootOf(m, byId);
    if (root === m.id) continue;
    const entry = stats.get(root) || { replies: 0, lastAt: 0 };
    entry.replies += 1;
    entry.lastAt = Math.max(entry.lastAt, Date.parse(m.created_at) || 0);
    stats.set(root, entry);
  }
  const roots = new Set([...stats.keys(), ...metaByRoot.keys()].filter(id => byId.has(id)));
  const entries = [...roots].map(id => {
    const root = byId.get(id);
    const meta = metaByRoot.get(id) || null;
    const { replies, lastAt } = stats.get(id) || { replies: 0, lastAt: 0 };
    return {
      id,
      root,
      name: threadName(root, meta, plain),
      named: Boolean(meta && meta.name),
      replies,
      lastAt: lastAt || Date.parse(root.created_at) || 0,
      archived: Boolean(meta && meta.archived),
      locked: Boolean(meta && meta.locked),
    };
  });
  entries.sort((a, b) => b.lastAt - a.lastAt || a.name.localeCompare(b.name));
  return { active: entries.filter(e => !e.archived), archived: entries.filter(e => e.archived) };
}

function canManageThread(root, { userId, canManageMessages } = {}) {
  return Boolean(root) && (Boolean(canManageMessages) || (Boolean(userId) && root.author_id === userId));
}

function threadComposerState({ meta, canManageMessages } = {}) {
  if (!meta || !meta.locked) return { disabled: false, placeholder: "Reply in thread" };
  if (canManageMessages) return { disabled: false, placeholder: "Reply in thread (locked for members)" };
  return { disabled: true, placeholder: "This thread is locked" };
}

function threadStatusLine({ meta, replies } = {}) {
  const parts = [];
  if (meta && meta.locked) parts.push("Locked");
  if (meta && meta.archived) parts.push("Archived");
  const n = Number(replies) || 0;
  parts.push(n ? `${n} ${n === 1 ? "reply" : "replies"}` : "No replies yet");
  return parts.join(" · ");
}

function threadsButtonState({ view, channel, count } = {}) {
  const visible = view === "server" && Boolean(channel) && (channel.kind === "text" || channel.kind === "announcement");
  const n = Math.max(0, Number(count) || 0);
  return { visible, count: n, title: n ? `${n} active ${n === 1 ? "thread" : "threads"}` : "Threads" };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    MAX_THREAD_NAME_CHARS, threadRootOf, threadReplyCounts, defaultThreadName, threadName, cleanThreadName,
    buildThreadList, canManageThread, threadComposerState, threadStatusLine, threadsButtonState,
  };
}
