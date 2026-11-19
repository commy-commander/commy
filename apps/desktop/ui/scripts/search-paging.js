const SEARCH_PAGE_SIZE = 50;

const SEARCH_SCOPES = [
  { key: "conversation", get label() { return t("ui.this-conversation"); } },
  { key: "dms", get label() { return t("ui.all-direct-messages"); } },
  { key: "server", get label() { return t("ui.this-server"); } },
];

function scopeEnabled(key, context) {
  const ctx = context || {};
  if (key === "server") return ctx.view === "server" && !ctx.group;
  if (key === "conversation") return Boolean(ctx.conversation);
  return true;
}

function searchScopes(context) {
  return SEARCH_SCOPES.map(scope => ({ ...scope, enabled: scopeEnabled(scope.key, context) }));
}

function defaultSearchScope(context) {
  const ctx = context || {};
  if (ctx.view === "server" && !ctx.group) return "server";
  return ctx.conversation ? "conversation" : "dms";
}

function pickSearchScope(current, context) {
  return current && scopeEnabled(current, context) ? current : defaultSearchScope(context);
}

function searchPlace(context) {
  const ctx = context || {};
  return `${ctx.view || ""}:${ctx.serverId || ""}:${ctx.dmUser || ""}:${ctx.conversation ? 1 : 0}`;
}

function searchScopeTitle(scope, context) {
  if (scope === "dms") return t("ui.search-your-direct-messages");
  if (scope === "server") return t("ui.search-this-server");
  const ctx = context || {};
  return ctx.view === "server" && !ctx.group ? t("ui.search-this-channel") : t("ui.search-this-conversation");
}

function searchScopeLine(scope, query, context) {
  const ctx = context || {};
  if (scope === "dms") return t("ui.newest-first-across-your-direct-messages");
  if (scope === "server") return query && query.channel ? t("ui.in", { name: query.channel.name }) : t("ui.newest-first-across-this-server");
  if (ctx.view === "dm") return ctx.conversationName ? t("ui.with", { conversationName: ctx.conversationName }) : t("ui.in-this-conversation");
  if (ctx.group) return ctx.conversationName ? t("ui.in-2", { conversationName: ctx.conversationName }) : t("ui.in-this-group");
  const channel = query && query.channel ? query.channel.name : ctx.conversationName;
  return channel ? t("ui.in-3", { channel }) : t("ui.in-this-channel");
}

function searchRequest(scope, args, context, offset) {
  const ctx = context || {};
  const at = Math.max(0, Math.floor(Number(offset) || 0));
  const dmArgs = { terms: args.terms, from: args.from, mentions: args.mentions, userId: null, offset: at };
  if (scope === "dms") return { command: "search_dms", args: dmArgs };
  if (scope === "conversation" && ctx.view === "dm") return { command: "search_dms", args: { ...dmArgs, userId: ctx.dmUser } };
  const channelId = scope === "conversation" && !ctx.group ? args.channelId || ctx.channelId : args.channelId;
  return { command: "search_messages", args: { ...args, serverId: ctx.serverId, channelId: channelId || null, offset: at } };
}

function clampSearchOffset(total, offset, size = SEARCH_PAGE_SIZE) {
  const n = Math.max(0, Math.floor(Number(total) || 0));
  const at = Math.max(0, Math.floor(Number(offset) || 0));
  if (!n) return 0;
  return Math.min(at, Math.floor((n - 1) / size) * size);
}

function pageWindow(total, offset, size = SEARCH_PAGE_SIZE) {
  const n = Math.max(0, Math.floor(Number(total) || 0));
  const at = clampSearchOffset(n, offset, size);
  const last = Math.min(n, at + size);
  return {
    total: n,
    offset: at,
    size,
    first: n ? at + 1 : 0,
    last,
    hasOlder: last < n,
    hasNewer: at > 0,
    olderOffset: last < n ? at + size : at,
    newerOffset: Math.max(0, at - size),
  };
}

function pageCount(total) {
  const n = Math.max(0, Math.floor(Number(total) || 0));
  if (!n) return t("ui.no-messages-found");
  return t(n === 1 ? "ui.n.results.one" : "ui.n.results.many", { count: n });
}

function pageLabel(total, offset, size = SEARCH_PAGE_SIZE) {
  const page = pageWindow(total, offset, size);
  if (page.total <= page.size) return pageCount(page.total);
  return t("ui.showing-to-of", { first: page.first, last: page.last, total: page.total });
}

function dmHitKey(hit) {
  if (!hit) return { key: "", label: "" };
  if (hit.kind === "group") return { key: `group:${hit.server}`, label: hit.name || t("ui.group") };
  return { key: `dm:${hit.user}`, label: `@${hit.name || "Unknown User"}` };
}

function groupHits(hits, keyOf = dmHitKey) {
  const runs = [];
  for (const hit of hits || []) {
    const { key, label } = keyOf(hit);
    const last = runs[runs.length - 1];
    if (last && last.key === key) last.hits.push(hit);
    else runs.push({ key, label, hits: [hit] });
  }
  return runs;
}

function searchHitMessage(hit) {
  const message = (hit && hit.message) || {};
  if (!hit || hit.kind !== "dm") return message;
  return {
    id: message.id,
    author_id: message.from,
    author_name: message.from_name,
    content: message.content,
    created_at: message.created_at,
    attachments: message.attachments || [],
  };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    SEARCH_PAGE_SIZE, SEARCH_SCOPES, searchScopes, defaultSearchScope, pickSearchScope, searchPlace, searchScopeTitle, searchScopeLine,
    searchRequest, clampSearchOffset, pageWindow, pageCount, pageLabel, dmHitKey, groupHits, searchHitMessage,
  };
}
