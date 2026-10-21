"use strict";

const BOT_GUIDE_URL = "https://github.com/commy-commander/commy/blob/main/crates/commy-bot/README.md";
const BOT_PORTING_URL = `${BOT_GUIDE_URL}#porting-a-bot`;
const BOT_DEFAULT_NAME = "Helper";
const MAX_BOT_NAME_CHARS = 32;

const BOT_ABILITIES = [
  ["manage_messages", "remove messages"],
  ["kick_members", "kick members"],
  ["ban_members", "ban members"],
  ["time_out_members", "time out members"],
  ["manage_roles", "change roles"],
  ["manage_channels", "manage channels"],
  ["manage_server", "change server settings"],
];

function botPermissions(server, member) {
  const held = new Set(member.roles || []);
  const granted = new Set();
  for (const role of server.roles || []) {
    if (!held.has(role.id)) continue;
    for (const p of role.permissions || []) granted.add(p);
  }
  return granted;
}

function joinWords(words) {
  if (words.length <= 1) return words.join("");
  return `${words.slice(0, -1).join(", ")} and ${words[words.length - 1]}`;
}

function botAbilityText(permissions) {
  if (permissions.has("administrator")) return "Can do anything in this server (Administrator).";
  const can = BOT_ABILITIES.filter(([key]) => permissions.has(key)).map(([, words]) => words);
  if (!can.length) return "Can read and post like a member. Give it a role to let it moderate.";
  const text = `Can ${joinWords(can)}.`;
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function botCommandsFor(server, botId) {
  const group = (server.commands || []).find(g => g.bot === botId);
  return group ? group.commands.map(c => ({ name: c.name, description: c.description || "" })) : [];
}

function serverBots(server) {
  return (server.members || [])
    .filter(m => m.bot)
    .map(member => {
      const permissions = botPermissions(server, member);
      return {
        member,
        online: Boolean(member.presence) && member.presence !== "offline" && member.presence !== "invisible",
        roles: (server.roles || []).filter(r => (member.roles || []).includes(r.id)),
        abilities: botAbilityText(permissions),
        commands: botCommandsFor(server, member.id),
      };
    })
    .sort((a, b) => Number(b.online) - Number(a.online) || a.member.name.localeCompare(b.member.name));
}

function canSeeBotsPage(server) {
  return Boolean(server && (server.can_manage_server || server.can_manage_roles));
}

function cleanBotName(name) {
  const cleaned = String(name || "")
    .replace(/[^\p{L}\p{N} ._-]/gu, "")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, MAX_BOT_NAME_CHARS)
    .trim();
  return cleaned || BOT_DEFAULT_NAME;
}

function quoteArg(text) {
  return /^[A-Za-z0-9._-]+$/.test(text) ? text : `"${text}"`;
}

function botSetupCommands(name, invite) {
  return {
    create: `commy-bot init ${quoteArg(cleanBotName(name))}`,
    join: `commy-bot join ${invite || "INVITE-CODE"}`,
    run: "commy-bot run",
    bridge: "commy-bot run --bridge",
  };
}

function newBots(knownIds, server) {
  const known = new Set(knownIds);
  return serverBots(server).filter(b => !known.has(b.member.id));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    BOT_GUIDE_URL,
    BOT_PORTING_URL,
    BOT_DEFAULT_NAME,
    MAX_BOT_NAME_CHARS,
    botPermissions,
    botAbilityText,
    botCommandsFor,
    serverBots,
    canSeeBotsPage,
    cleanBotName,
    botSetupCommands,
    newBots,
  };
}
