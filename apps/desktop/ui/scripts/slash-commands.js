"use strict";

const COMMAND_MIME = "application/vnd.commy.command+json";
const MAX_SLASH_COMMANDS = 25;
const MAX_SLASH_NAME_CHARS = 32;
const MAX_SLASH_DESCRIPTION_CHARS = 100;
const MAX_SLASH_OPTIONS = 10;
const SLASH_SUGGEST_LIMIT = 8;
const SLASH_NAME = /^[a-z0-9](?:[a-z0-9-]{0,30}[a-z0-9])?$/;

function validSlashName(name) {
  return typeof name === "string" && name.length <= MAX_SLASH_NAME_CHARS && SLASH_NAME.test(name);
}

function slashCatalog(server) {
  const out = [];
  for (const group of (server && server.commands) || []) {
    for (const command of (group.commands || []).slice(0, MAX_SLASH_COMMANDS)) {
      if (!validSlashName(command.name)) continue;
      out.push({
        bot: group.bot,
        bot_name: group.bot_name || "a bot",
        name: command.name,
        description: String(command.description || "").slice(0, MAX_SLASH_DESCRIPTION_CHARS),
        options: (command.options || []).slice(0, MAX_SLASH_OPTIONS).map(o => ({
          name: String(o.name || ""),
          description: String(o.description || ""),
          required: Boolean(o.required),
          kind: ["string", "integer", "user", "channel"].includes(o.kind) ? o.kind : "string",
        })),
      });
    }
  }
  return out.sort((a, b) => a.name.localeCompare(b.name) || a.bot_name.localeCompare(b.bot_name));
}

function slashNames(catalog) {
  return [...new Set((catalog || []).map(c => c.name))];
}

function slashSuggestions(query, catalog, limit = SLASH_SUGGEST_LIMIT) {
  const q = String(query || "").toLowerCase();
  return (catalog || [])
    .filter(c => c.name.startsWith(q))
    .sort((a, b) => Number(b.name === q) - Number(a.name === q) || a.name.localeCompare(b.name) || a.bot_name.localeCompare(b.bot_name))
    .slice(0, limit);
}

function matchSlashCommand(text, catalog) {
  const m = String(text || "").match(/^\/([^\s]+)(?:\s+([^]*))?$/);
  if (!m) return null;
  const name = m[1].toLowerCase();
  const entry = (catalog || []).find(c => c.name === name);
  return entry ? { entry, rest: m[2] || "" } : null;
}

function splitSlashArgs(rest) {
  const tokens = [];
  const re = /"([^"]*)"?|(\S+)/g;
  let m;
  while ((m = re.exec(String(rest || "")))) tokens.push({ value: m[1] !== undefined ? m[1] : m[2], quoted: m[1] !== undefined, index: m.index });
  return tokens;
}

function optionPlaceholder(option) {
  return option.required ? `<${option.name}>` : `[${option.name}]`;
}

function slashUsage(entry) {
  return [`/${entry.name}`, ...entry.options.map(optionPlaceholder)].join(" ");
}

function slashOptionAt(entry, rest) {
  const text = String(rest || "");
  const tokens = splitSlashArgs(text);
  if (!entry.options.length) return -1;
  const typing = tokens.length && !/\s$/.test(text) ? tokens.length - 1 : tokens.length;
  return Math.min(typing, entry.options.length - 1);
}

function slashHint(entry, rest) {
  const at = slashOptionAt(entry, rest);
  if (at < 0) return { usage: slashUsage(entry), at, text: entry.description };
  const option = entry.options[at];
  const kind = { string: "text", integer: "a whole number", user: "@member", channel: "#channel" }[option.kind];
  return { usage: slashUsage(entry), at, text: `${option.description || option.name} (${kind}${option.required ? "" : ", optional"})` };
}

function findByName(list, raw, key = "name") {
  const want = String(raw || "").toLowerCase();
  return (list || []).find(item => String(item[key] || "").toLowerCase() === want) || null;
}

function parseSlashOptions(entry, rest, { members = [], channels = [] } = {}) {
  const text = String(rest || "");
  const tokens = splitSlashArgs(text);
  const options = {};
  const slots = entry.options;
  if (!slots.length && text.trim()) return { options, problem: `/${entry.name} takes no options` };
  for (let i = 0; i < slots.length; i++) {
    const option = slots[i];
    const last = i === slots.length - 1;
    let token = tokens[i];
    if (last && option.kind === "string" && token && !token.quoted) token = { value: text.slice(token.index).trim(), quoted: false };
    if (!token) {
      if (option.required) return { options, problem: `${optionPlaceholder(option)} is needed: ${slashUsage(entry)}` };
      continue;
    }
    const value = token.value;
    if (option.kind === "integer") {
      if (!/^-?\d{1,18}$/.test(value)) return { options, problem: `${option.name} needs a whole number` };
      options[option.name] = String(Number(value));
    } else if (option.kind === "user") {
      const name = value.replace(/^@/, "");
      const member = findByName(members, name) || findByName(members, name, "username");
      if (!member) return { options, problem: `${option.name} needs a member of this server, written as @name` };
      options[option.name] = member.id;
    } else if (option.kind === "channel") {
      const channel = findByName(channels.filter(c => c.kind !== "category" && c.kind !== "voice"), value.replace(/^#/, ""));
      if (!channel) return { options, problem: `${option.name} needs a text channel, written as #name` };
      options[option.name] = channel.id;
    } else if (value) {
      options[option.name] = value;
    }
  }
  if (tokens.length > slots.length && !(slots.length && slots[slots.length - 1].kind === "string")) {
    return { options, problem: `/${entry.name} takes ${slots.length === 1 ? "one option" : `${slots.length} options`}: ${slashUsage(entry)}` };
  }
  return { options, problem: null };
}

function slashChips(command, entry, { memberName = () => null, channelName = () => null } = {}) {
  const options = (command && command.options) || {};
  const order = entry ? entry.options.map(o => o.name) : [];
  const names = [...order.filter(n => n in options), ...Object.keys(options).filter(n => !order.includes(n)).sort()];
  return names.map(name => {
    const kind = entry && entry.options.find(o => o.name === name) ? entry.options.find(o => o.name === name).kind : "string";
    const raw = String(options[name]);
    let value = raw;
    if (kind === "user") value = `@${memberName(raw) || "someone"}`;
    if (kind === "channel") value = `#${channelName(raw) || "a channel"}`;
    return { name, value };
  });
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    COMMAND_MIME,
    MAX_SLASH_COMMANDS,
    MAX_SLASH_NAME_CHARS,
    MAX_SLASH_DESCRIPTION_CHARS,
    MAX_SLASH_OPTIONS,
    SLASH_SUGGEST_LIMIT,
    validSlashName,
    slashCatalog,
    slashNames,
    slashSuggestions,
    matchSlashCommand,
    splitSlashArgs,
    slashUsage,
    slashOptionAt,
    slashHint,
    parseSlashOptions,
    slashChips,
  };
}
