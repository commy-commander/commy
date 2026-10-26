"use strict";

const SUGGEST_LIMIT = 8;
const COMPOSER_COMMANDS = [{ name: "gif", needs: "gifs" }, { name: "tts", needs: "tts" }];

function suggestTokenAt(value, caret, names = []) {
  const text = String(value || "");
  const before = text.slice(0, caret ?? text.length);
  const command = before.match(/^\/([a-z0-9-]{0,32})(?:\s+([^]*))?$/i);
  const mention = before.match(/(^|\s)@([^\s@]{0,32})$/);
  if (command) {
    const name = command[1].toLowerCase();
    const builtin = COMPOSER_COMMANDS.some(c => c.name === name);
    const bot = names.includes(name);
    if (command[2] === undefined || builtin || (bot && !mention)) {
      return { kind: "command", start: 0, query: name, args: command[2] ?? null };
    }
  }
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
    for (const face of group.emoticons || []) {
      if (seen.has(`emoticon:${face.name}`)) continue;
      const rank = emojiNameRank(face.name, query);
      if (rank < 0) continue;
      seen.add(`emoticon:${face.name}`);
      matches.push({ kind: "emoticon", name: face.name, text: face.text, server: group.name, rank, order });
    }
  });
  return matches.sort((a, b) => a.rank - b.rank || a.order - b.order || a.name.localeCompare(b.name));
}

function standardEmojiNames(row) {
  const [, label, codes] = row;
  return [...(codes ? codes.split(" ") : []), label.replace(/\s+/g, "_")];
}

function standardEmojiMatches(query, standard) {
  const matches = [];
  standard.forEach((row, order) => {
    let best = null;
    for (const code of standardEmojiNames(row)) {
      const rank = emojiNameRank(code, query);
      if (rank >= 0 && (!best || rank < best.rank)) best = { code, rank };
    }
    if (best) matches.push({ kind: "standard", emoji: row[0], name: row[1], code: best.code, skin: row[6] || 0, rank: best.rank, order });
  });
  return matches.sort((a, b) => a.rank - b.rank || a.order - b.order);
}

function emojiSuggestions(query, groups, standard, limit = SUGGEST_LIMIT) {
  const q = String(query || "").toLowerCase();
  if (q.length < 2) return [];
  return [...customEmojiMatches(q, groups || []), ...standardEmojiMatches(q, standard || [])].slice(0, limit);
}

function commandSuggestions(token, available = {}, catalog = []) {
  const typed = token.args !== null;
  const builtin = COMPOSER_COMMANDS
    .filter(c => (typed ? c.name === token.query : c.name.startsWith(token.query)))
    .map(c => ({
      kind: "command",
      name: c.name,
      args: typed ? token.args.trim() : "",
      ready: c.name === token.query,
      available: Boolean(available[c.needs]),
    }));
  const bots = (catalog || [])
    .filter(c => (typed ? c.name === token.query : c.name.startsWith(token.query)))
    .map(c => ({ kind: "command", bot: true, name: c.name, args: typed ? token.args : null, ready: c.name === token.query, entry: c }));
  return [...builtin, ...bots].slice(0, SUGGEST_LIMIT);
}

function suggestionInsert(item) {
  if (item.kind === "custom") return `:${item.name}: `;
  if (item.kind === "emoticon") return `${item.text} `;
  if (item.kind === "standard") return `${item.emoji} `;
  if (item.kind === "command") return `/${item.name} `;
  return `@${item.name} `;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { SUGGEST_LIMIT, COMPOSER_COMMANDS, suggestTokenAt, emojiSuggestions, commandSuggestions, suggestionInsert };
}
