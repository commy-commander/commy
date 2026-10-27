const SWITCHER_RECENT_CAP = 20;
const SWITCHER_CHANNEL_KINDS = new Set(["text", "voice", "announcement", "forum", "stage", "drive"]);
const SWITCHER_PREFIXES = { "#": "channel", "@": "person", "*": "server" };

function parseSwitcherQuery(raw) {
  const trimmed = String(raw || "").trim();
  const kind = SWITCHER_PREFIXES[trimmed[0]] || null;
  const text = (kind ? trimmed.slice(1) : trimmed).trim().toLowerCase();
  return { kind, text };
}

function matchTier(needle, haystack) {
  if (!needle) return 0;
  needle = needle.toLowerCase();
  const text = haystack.toLowerCase().replace(/^#/, "");
  const extra = Math.max(0, text.length - needle.length);
  if (text.startsWith(needle)) return 3000 + Math.max(0, 999 - extra);
  const at = text.indexOf(needle);
  if (at >= 0) return 2000 + Math.max(0, 999 - at * 10 - extra);
  let from = 0;
  let first = -1;
  let last = -1;
  for (const ch of needle) {
    const hit = text.indexOf(ch, from);
    if (hit < 0) return -1;
    if (first < 0) first = hit;
    last = hit;
    from = hit + 1;
  }
  return 1000 + Math.max(0, 999 - (last - first) * 4 - first - extra);
}

function switcherScore(query, item) {
  const own = matchTier(query, item.name || "");
  if (own >= 0) return own;
  if (!item.subtitle) return -1;
  const sub = matchTier(query, item.subtitle);
  if (sub < 0) return -1;
  return (Math.floor(sub / 1000) - 1) * 333 + Math.floor((sub % 1000) / 3);
}

function rankSwitcher(items, rawQuery, recent = []) {
  const { kind, text } = parseSwitcherQuery(rawQuery);
  const pool = kind ? items.filter(i => i.kind === kind) : items;
  const recency = new Map(recent.map((key, at) => [key, at]));
  const place = item => (recency.has(item.key) ? recency.get(item.key) : Infinity);
  if (!text) {
    return pool
      .map((item, at) => ({ item, at }))
      .sort((a, b) => place(a.item) - place(b.item) || a.at - b.at)
      .map(({ item }) => item);
  }
  return pool
    .map((item, at) => ({ item, at, score: switcherScore(text, item) }))
    .filter(x => x.score >= 0)
    .sort((a, b) => b.score - a.score || place(a.item) - place(b.item) || a.at - b.at)
    .map(({ item }) => item);
}

function rememberRecent(recent, key, cap = SWITCHER_RECENT_CAP) {
  return [key, ...(recent || []).filter(k => k !== key)].slice(0, cap);
}

function readRecent(raw) {
  try {
    const parsed = JSON.parse(raw || "[]");
    return Array.isArray(parsed) ? parsed.filter(k => typeof k === "string").slice(0, SWITCHER_RECENT_CAP) : [];
  } catch {
    return [];
  }
}

function switcherChannelLabel(channel, plainName) {
  const name = plainName ? plainName(channel) : channel.name;
  return channel.kind === "voice" || channel.kind === "stage" ? name : `#${name}`;
}

function buildSwitcherItems({ servers = [], views = {}, threads = [], friends = [], presence = () => "", plainName = null, actions = [] }) {
  const items = [];
  const groups = servers.filter(s => s.group);
  for (const summary of servers) {
    if (summary.group) continue;
    items.push({ key: `server:${summary.id}`, kind: "server", name: summary.name, subtitle: "Server", serverId: summary.id });
  }
  for (const summary of servers) {
    const view = views[summary.id];
    if (!view) continue;
    for (const channel of view.channels || []) {
      if (!SWITCHER_CHANNEL_KINDS.has(channel.kind) || channel.can_view === false) continue;
      items.push({
        key: `channel:${channel.id}`, kind: "channel", name: switcherChannelLabel(channel, plainName), subtitle: summary.name,
        serverId: summary.id, channelId: channel.id, channelKind: channel.kind,
      });
    }
  }
  const seen = new Set();
  for (const thread of threads) {
    if (thread.kind === "group") {
      const group = groups.find(s => s.id === thread.user);
      items.push({ key: `group:${thread.user}`, kind: "person", name: thread.name, subtitle: "Group", serverId: thread.user, group: true, members: group ? group.member_count : 0 });
      continue;
    }
    seen.add(thread.user);
    items.push({ key: `dm:${thread.user}`, kind: "person", name: thread.name, subtitle: presence(thread.user) || "Direct message", userId: thread.user });
  }
  for (const friend of friends) {
    if (seen.has(friend.id)) continue;
    seen.add(friend.id);
    items.push({ key: `dm:${friend.id}`, kind: "person", name: friend.name, subtitle: presence(friend.id) || "Friend", userId: friend.id });
  }
  for (const action of actions) {
    items.push({ key: `action:${action.id}`, kind: "action", name: action.name, subtitle: action.subtitle || "", action: action.id });
  }
  return items;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    SWITCHER_RECENT_CAP, parseSwitcherQuery, switcherScore, rankSwitcher, rememberRecent, readRecent, buildSwitcherItems,
  };
}
