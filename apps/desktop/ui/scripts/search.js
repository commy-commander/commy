const SEARCH_FILTERS = [
  { key: "from", prefix: "@", kind: "member", hint: "messages by a member" },
  { key: "mentions", prefix: "@", kind: "member", hint: "messages that mention a member" },
  { key: "in", prefix: "#", kind: "channel", hint: "messages in one channel" },
];

const SEARCH_SUGGEST_LIMIT = 8;
const SEARCH_EXCERPT_CHARS = 220;

function searchFilterFor(key) {
  return SEARCH_FILTERS.find(f => f.key === key.toLowerCase()) || null;
}

function isSearchSpace(ch) {
  return /\s/.test(ch);
}

function readSearchWord(text, at) {
  let end = at;
  while (end < text.length && !isSearchSpace(text[end])) end++;
  return { value: text.slice(at, end), end };
}

function readSearchQuoted(text, at) {
  const close = text.indexOf('"', at + 1);
  const end = close === -1 ? text.length : close + 1;
  return { value: text.slice(at + 1, close === -1 ? text.length : close), end };
}

function longestSearchNameAt(text, at, candidates) {
  const rest = text.slice(at).toLowerCase();
  let best = null;
  for (const item of candidates) {
    const name = (item.name || "").toLowerCase();
    if (!name || !rest.startsWith(name)) continue;
    const next = rest[name.length];
    if (next !== undefined && !isSearchSpace(next) && !/[,.!?;:]/.test(next)) continue;
    if (!best || name.length > best.name.length) best = item;
  }
  return best ? { item: best, end: at + best.name.length } : null;
}

function readSearchTarget(text, at, candidates) {
  if (text[at] === '"') {
    const quoted = readSearchQuoted(text, at);
    const lower = quoted.value.toLowerCase();
    return { item: candidates.find(c => (c.name || "").toLowerCase() === lower) || null, raw: quoted.value, end: quoted.end };
  }
  const named = longestSearchNameAt(text, at, candidates);
  if (named) return { item: named.item, raw: named.item.name, end: named.end };
  const word = readSearchWord(text, at);
  return { item: null, raw: word.value, end: word.end };
}

function parseSearch(text, { members = [], channels = [] } = {}) {
  const query = { terms: [], from: null, mentions: null, channel: null, unknown: [] };
  const source = String(text || "");
  let at = 0;
  while (at < source.length) {
    if (isSearchSpace(source[at])) {
      at++;
      continue;
    }
    if (source[at] === '"') {
      const quoted = readSearchQuoted(source, at);
      if (quoted.value.trim()) query.terms.push(quoted.value.trim());
      at = quoted.end;
      continue;
    }
    const filter = /^(from|mentions|in):/i.exec(source.slice(at));
    const bareMention = !filter && source[at] === "@" && at + 1 < source.length && !isSearchSpace(source[at + 1]);
    if (filter || bareMention) {
      const spec = filter ? searchFilterFor(filter[1]) : searchFilterFor("mentions");
      let start = at + (filter ? filter[0].length : 1);
      if (filter && source[start] === spec.prefix) start++;
      const candidates = spec.kind === "member" ? members : channels;
      const target = readSearchTarget(source, start, candidates);
      if (target.item) {
        query[spec.key === "in" ? "channel" : spec.key] = { id: target.item.id, name: target.item.name };
      } else if (bareMention) {
        query.terms.push(`@${target.raw}`);
      } else if (target.raw) {
        query.unknown.push({ filter: spec.key, name: target.raw });
      }
      at = Math.max(target.end, start);
      continue;
    }
    const word = readSearchWord(source, at);
    query.terms.push(word.value);
    at = word.end;
  }
  return query;
}

function searchIsEmpty(query) {
  return !query || (!query.terms.length && !query.from && !query.mentions && !query.channel);
}

function searchArgs(query) {
  return {
    terms: query.terms,
    from: query.from ? query.from.id : null,
    mentions: query.mentions ? query.mentions.id : null,
    channelId: query.channel ? query.channel.id : null,
  };
}

function searchToken(text, caret) {
  const before = String(text || "").slice(0, caret);
  const filter = /(^|\s)(from|mentions|in):([@#]?)([^:"]*)$/i.exec(before);
  if (filter) {
    const spec = searchFilterFor(filter[2]);
    return { filter: spec.key, kind: spec.kind, query: filter[4].toLowerCase(), start: filter.index + filter[1].length, end: caret };
  }
  const bare = /(^|\s)@([^@:"\s]*)$/.exec(before);
  if (bare) return { filter: "mentions", kind: "member", query: bare[2].toLowerCase(), start: bare.index + bare[1].length, end: caret, bare: true };
  return null;
}

function searchSuggestions(token, { members = [], channels = [] } = {}) {
  if (!token) return [];
  const pool = token.kind === "member" ? members : channels;
  const q = token.query.trim();
  return pool
    .filter(c => c.name && c.name.toLowerCase().includes(q))
    .sort((a, b) => {
      const ap = a.name.toLowerCase().startsWith(q) ? 0 : 1;
      const bp = b.name.toLowerCase().startsWith(q) ? 0 : 1;
      return ap - bp || a.name.localeCompare(b.name);
    })
    .slice(0, SEARCH_SUGGEST_LIMIT);
}

function applySearchSuggestion(text, token, item) {
  const spec = searchFilterFor(token.filter);
  const head = token.bare ? "@" : `${spec.key}:${spec.prefix}`;
  const insert = `${head}${item.name} `;
  const value = String(text || "");
  const next = value.slice(0, token.start) + insert + value.slice(token.end).replace(/^\s+/, "");
  return { text: next, caret: token.start + insert.length };
}

function applySearchFilter(text, key) {
  const spec = searchFilterFor(key);
  const value = String(text || "").replace(/\s+$/, "");
  const next = `${value}${value ? " " : ""}${spec.key}:${spec.prefix}`;
  return { text: next, caret: next.length };
}

function highlightParts(text, terms) {
  const source = String(text || "");
  const lower = source.toLowerCase();
  const ranges = [];
  for (const term of terms || []) {
    const needle = String(term).toLowerCase();
    if (!needle) continue;
    let from = 0;
    let found;
    while ((found = lower.indexOf(needle, from)) !== -1) {
      ranges.push([found, found + needle.length]);
      from = found + needle.length;
    }
  }
  ranges.sort((a, b) => a[0] - b[0]);
  const merged = [];
  for (const range of ranges) {
    const last = merged[merged.length - 1];
    if (last && range[0] <= last[1]) last[1] = Math.max(last[1], range[1]);
    else merged.push([...range]);
  }
  const parts = [];
  let at = 0;
  for (const [start, end] of merged) {
    if (start > at) parts.push({ text: source.slice(at, start), hit: false });
    parts.push({ text: source.slice(start, end), hit: true });
    at = end;
  }
  if (at < source.length) parts.push({ text: source.slice(at), hit: false });
  return parts;
}

function searchExcerpt(text, terms, max = SEARCH_EXCERPT_CHARS) {
  const source = String(text || "").replace(/\s+/g, " ").trim();
  if (source.length <= max) return source;
  const lower = source.toLowerCase();
  const first = Math.min(...(terms || []).map(t => lower.indexOf(String(t).toLowerCase())).filter(i => i >= 0), Infinity);
  if (first === Infinity || first < max * 0.6) return `${source.slice(0, max).trimEnd()}…`;
  const start = Math.max(0, first - Math.floor(max / 3));
  const end = Math.min(source.length, start + max);
  return `…${source.slice(start, end).trim()}${end < source.length ? "…" : ""}`;
}

function searchSummary(total, shown) {
  const n = Number(total) || 0;
  if (n === 0) return "No messages found";
  const label = n === 1 ? "1 result" : `${n} results`;
  return n > shown ? `${label}, showing the newest ${shown}` : label;
}

function unknownFilterText(unknown, where = "in this server") {
  if (!unknown || !unknown.length) return "";
  const first = unknown[0];
  return first.filter === "in" ? `No channel called #${first.name} here` : `Nobody called @${first.name} ${where}`;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    SEARCH_FILTERS, parseSearch, searchIsEmpty, searchArgs, searchToken, searchSuggestions,
    applySearchSuggestion, applySearchFilter, highlightParts, searchExcerpt, searchSummary, unknownFilterText,
  };
}
