"use strict";

const EMOJI_FIELDS = { emoji: 0, label: 1, codes: 2, keywords: 3, group: 4, version: 5, skin: 6 };
const SKIN_FIRST = 1;
const SKIN_EVERY_PERSON = 2;
const PERSON_BASES = new Set([0x1F9D1, 0x1F468, 0x1F469, 0x1F9D2, 0x1F466, 0x1F467]);
const VARIATION_SELECTOR = 0xFE0F;
const FREQUENT_LIMIT = 27;
const FREQUENT_KEEP = 60;
const SEARCH_LIMIT = 300;
const LEADING_EMOJI_RE = /^(?:[\u{1F1E6}-\u{1F1FF}]{2}|[#*0-9]\uFE0F?\u20E3|\p{Extended_Pictographic}(?:\uFE0F|[\u{1F3FB}-\u{1F3FF}])*(?:\u200D\p{Extended_Pictographic}(?:\uFE0F|[\u{1F3FB}-\u{1F3FF}])*)*|\p{Emoji_Presentation})/u;
const LEADING_CUSTOM_RE = /^:([a-z0-9_]{2,32}):/;
const NAME_SEPARATOR_RE = /^[-\s]/;

function emojiRow(row) {
  return {
    emoji: row[EMOJI_FIELDS.emoji],
    label: row[EMOJI_FIELDS.label],
    codes: row[EMOJI_FIELDS.codes] ? row[EMOJI_FIELDS.codes].split(" ") : [],
    keywords: row[EMOJI_FIELDS.keywords] ? row[EMOJI_FIELDS.keywords].split(" ") : [],
    group: row[EMOJI_FIELDS.group],
    version: row[EMOJI_FIELDS.version] || 0,
    skin: row[EMOJI_FIELDS.skin] || 0,
  };
}

function toneAfter(points, skin) {
  if (skin === SKIN_FIRST) return i => i === 0;
  if (skin === SKIN_EVERY_PERSON) return i => i === 0 || PERSON_BASES.has(points[i]);
  return () => false;
}

function withSkinTone(emoji, skin, tone, tones) {
  const modifier = tone > 0 && tones ? tones[tone - 1] : null;
  if (!modifier || !skin) return emoji;
  const mark = modifier.codePointAt(0);
  const points = Array.from(emoji, ch => ch.codePointAt(0));
  const toned = toneAfter(points, skin);
  const out = [];
  points.forEach((p, i) => {
    if (p === VARIATION_SELECTOR && i > 0 && toned(i - 1)) return;
    out.push(p);
    if (toned(i)) out.push(mark);
  });
  return String.fromCodePoint(...out);
}

function emojiQueryRank(entry, query) {
  const q = String(query || "").trim().toLowerCase().replace(/^:|:$/g, "");
  if (!q) return 0;
  const spaced = q.replace(/_/g, " ");
  if (entry.codes.includes(q)) return 0;
  if (entry.label.toLowerCase() === spaced) return 0;
  if (entry.codes.some(code => code.startsWith(q))) return 1;
  const words = entry.label.toLowerCase().split(/[\s:,]+/);
  if (entry.label.toLowerCase().startsWith(spaced)) return 2;
  if (words.some(word => word.startsWith(spaced))) return 3;
  if (entry.keywords.some(word => word === q)) return 4;
  if (entry.keywords.some(word => word.startsWith(q))) return 5;
  if (entry.label.toLowerCase().includes(spaced) || entry.codes.some(code => code.includes(q))) return 6;
  return -1;
}

function searchEmoji(rows, query, { maxVersion = Infinity, limit = SEARCH_LIMIT } = {}) {
  const found = [];
  rows.forEach((row, index) => {
    const entry = emojiRow(row);
    if (entry.version > maxVersion) return;
    const rank = emojiQueryRank(entry, query);
    if (rank >= 0) found.push({ entry, index, rank });
  });
  return found.sort((a, b) => a.rank - b.rank || a.index - b.index).slice(0, limit);
}

function emojiByGroup(rows, groups, { maxVersion = Infinity, hidden = [] } = {}) {
  const sections = groups.map((group, index) => ({ ...group, index, items: [] }));
  rows.forEach((row, index) => {
    const entry = emojiRow(row);
    if (entry.version > maxVersion || hidden.includes(groups[entry.group] && groups[entry.group].key)) return;
    if (sections[entry.group]) sections[entry.group].items.push({ entry, index });
  });
  return sections.filter(section => section.items.length);
}

function emojiCodesLabel(entry) {
  return (entry.codes.length ? entry.codes.slice(0, 4) : [entry.label.replace(/\s+/g, "_")]).map(code => `:${code}:`).join(" ");
}

function readFrequentEmoji(raw) {
  try {
    const parsed = JSON.parse(raw || "{}");
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    const out = {};
    for (const [key, value] of Object.entries(parsed)) {
      if (typeof key === "string" && key && value && Number.isFinite(value.n) && Number.isFinite(value.t)) out[key] = { n: value.n, t: value.t };
    }
    return out;
  } catch {
    return {};
  }
}

function recordEmojiUse(store, key, now) {
  const next = { ...store };
  const seen = next[key] || { n: 0, t: 0 };
  next[key] = { n: seen.n + 1, t: now };
  const keys = Object.keys(next);
  if (keys.length <= FREQUENT_KEEP) return next;
  const keep = new Set(rankFrequent(next).slice(0, FREQUENT_KEEP));
  keep.add(key);
  return Object.fromEntries(Object.entries(next).filter(([k]) => keep.has(k)));
}

function rankFrequent(store) {
  return Object.entries(store)
    .sort(([, a], [, b]) => b.n - a.n || b.t - a.t)
    .map(([key]) => key);
}

function frequentEmoji(store, usable, limit = FREQUENT_LIMIT) {
  return rankFrequent(store).filter(usable).slice(0, limit);
}

function emojiVersionProbes(rows) {
  const best = new Map();
  rows.forEach(row => {
    const entry = emojiRow(row);
    const points = Array.from(entry.emoji).filter(ch => ch.codePointAt(0) !== VARIATION_SELECTOR).length;
    const known = best.get(entry.version);
    if (!known || points < known.points) best.set(entry.version, { version: entry.version, emoji: entry.emoji, points });
  });
  return [...best.values()].sort((a, b) => b.version - a.version).map(({ version, emoji }) => ({ version, emoji }));
}

function highestSupportedVersion(probes, renders) {
  if (!probes.length) return Infinity;
  const oldest = probes[probes.length - 1];
  if (!renders(oldest.emoji)) return Infinity;
  for (const probe of probes) if (renders(probe.emoji)) return probe.version;
  return Infinity;
}

function splitLeadingEmoji(name, isCustom = () => false) {
  const text = String(name || "");
  const custom = LEADING_CUSTOM_RE.exec(text);
  let lead = "";
  if (custom && isCustom(custom[1])) lead = custom[0];
  else {
    const standard = LEADING_EMOJI_RE.exec(text);
    if (standard && !/^[#*0-9]$/.test(standard[0])) lead = standard[0];
  }
  if (!lead) return { lead: "", separator: null, rest: text };
  const after = text.slice(lead.length);
  const separator = NAME_SEPARATOR_RE.test(after) ? after[0] : "";
  return { lead, separator, rest: after.slice(separator.length) };
}

function joinLeadingEmoji(lead, rest, separator = "-") {
  const body = String(rest || "");
  if (!lead) return body;
  return body ? `${lead}${separator}${body}` : lead;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    SKIN_FIRST, SKIN_EVERY_PERSON, FREQUENT_LIMIT, FREQUENT_KEEP, SEARCH_LIMIT,
    emojiRow, withSkinTone, emojiQueryRank, searchEmoji, emojiByGroup, emojiCodesLabel,
    readFrequentEmoji, recordEmojiUse, frequentEmoji, emojiVersionProbes, highestSupportedVersion,
    splitLeadingEmoji, joinLeadingEmoji,
  };
}
