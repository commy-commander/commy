"use strict";

const SUGGEST_LIMIT = 8;
const COMPOSER_COMMANDS = [{ name: "gif", needs: "gifs" }];

function suggestTokenAt(value, caret) {
  const text = String(value || "");
  const before = text.slice(0, caret ?? text.length);
  const command = before.match(/^\/([a-z]{0,16})(?:\s+([^]*))?$/i);
  if (command && (command[2] === undefined || COMPOSER_COMMANDS.some(c => c.name === command[1].toLowerCase()))) {
    return { kind: "command", start: 0, query: command[1].toLowerCase(), args: command[2] ?? null };
  }
  const mention = before.match(/(^|\s)@([^\s@]{0,32})$/);
  if (mention) return { kind: "mention", start: before.length - mention[2].length - 1, query: mention[2].toLowerCase() };
  const emoji = before.match(/(?:^|\s):([a-z0-9_]{2,32})$/i);
  if (emoji) return { kind: "emoji", start: before.length - emoji[1].length - 1, query: emoji[1].toLowerCase() };
  return null;
}

function emojiNameRank(name, query) {
  if (name === query) return 0;
  if (name.startsWith(query)) return 1;
  if (name.split("_").some(word => word.startsWith(query))) return 2;
  return name.includes(query) ? 3 : -1;
}

function customEmojiMatches(query, groups) {
  const seen = new Set();
  const matches = [];
  groups.forEach((group, order) => {
    for (const emoji of group.emojis || []) {
      if (seen.has(emoji.name)) continue;
      const rank = emojiNameRank(emoji.name, query);
      if (rank < 0) continue;
      seen.add(emoji.name);
      matches.push({ kind: "custom", name: emoji.name, image: emoji.image, server: group.name, rank, order });
    }
  });
  return matches.sort((a, b) => a.rank - b.rank || a.order - b.order || a.name.localeCompare(b.name));
}

function standardEmojiMatches(query, standard) {
  const matches = [];
  standard.forEach(([emoji, label], order) => {
    const name = label.replace(/\s+/g, "_");
    const rank = emojiNameRank(name, query);
    if (rank >= 0) matches.push({ kind: "standard", emoji, name: label, rank, order });
  });
  return matches.sort((a, b) => a.rank - b.rank || a.order - b.order);
}

function emojiSuggestions(query, groups, standard, limit = SUGGEST_LIMIT) {
  const q = String(query || "").toLowerCase();
  if (q.length < 2) return [];
  return [...customEmojiMatches(q, groups || []), ...standardEmojiMatches(q, standard || [])].slice(0, limit);
}

function commandSuggestions(token, available = {}) {
  const commands = COMPOSER_COMMANDS.filter(c => available[c.needs]);
  const typed = token.args !== null;
  return commands
    .filter(c => (typed ? c.name === token.query : c.name.startsWith(token.query)))
    .map(c => ({ kind: "command", name: c.name, args: typed ? token.args.trim() : "", ready: c.name === token.query }));
}

function suggestionInsert(item) {
  if (item.kind === "custom") return `:${item.name}: `;
  if (item.kind === "standard") return `${item.emoji} `;
  if (item.kind === "command") return `/${item.name} `;
  return `@${item.name} `;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { SUGGEST_LIMIT, suggestTokenAt, emojiSuggestions, commandSuggestions, suggestionInsert };
}
