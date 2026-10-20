"use strict";

const ROLE_MENTION_RE = /<@&([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})>/gi;
const EVERYONE_MENTIONS = ["everyone", "here"];
const MENTION_EVERYONE_PERMISSION = "mention_everyone";
const UNKNOWN_ROLE_MENTION = "@role";

function escapeMentionName(name) {
  return name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function namesMentioned(content, names) {
  const escaped = (names || []).filter(Boolean).map(escapeMentionName);
  if (!content || !escaped.length) return false;
  return new RegExp(`(?<![\\p{L}\\p{N}_<])@(${escaped.join("|")})(?![\\p{L}\\p{N}_])`, "iu").test(content);
}

function mentionsEveryone(content) {
  return namesMentioned(content, EVERYONE_MENTIONS);
}

function isEveryoneMention(name) {
  return EVERYONE_MENTIONS.includes(String(name || "").toLowerCase());
}

function roleMentionToken(roleId) {
  return `<@&${roleId}>`;
}

function roleMentions(content) {
  const found = [];
  for (const [, id] of String(content || "").matchAll(ROLE_MENTION_RE)) {
    const lower = id.toLowerCase();
    if (!found.includes(lower)) found.push(lower);
  }
  return found;
}

function roleById(server, roleId) {
  const want = String(roleId || "").toLowerCase();
  return ((server && server.roles) || []).find(r => String(r.id).toLowerCase() === want) || null;
}

function serverMember(server, userId) {
  return ((server && server.members) || []).find(m => m.id === userId) || null;
}

function memberCanMentionEveryone(server, userId) {
  if (!server || !userId) return false;
  if (server.owner === userId) return true;
  const member = serverMember(server, userId);
  if (!member) return false;
  if (typeof member.can_mention_everyone === "boolean") return member.can_mention_everyone;
  const held = new Set(member.roles || []);
  return (server.roles || []).some(r => held.has(r.id)
    && (r.permissions || []).some(p => p === MENTION_EVERYONE_PERMISSION || p === "administrator"));
}

function roleMentionAllowed(server, authorId, role) {
  return Boolean(role && (role.mentionable || memberCanMentionEveryone(server, authorId)));
}

function mentionableRoles(server, userId) {
  const roles = (server && server.roles) || [];
  if (memberCanMentionEveryone(server, userId)) return roles.slice();
  return roles.filter(r => r.mentionable);
}

function messageMentionsMe({ content, server, authorId, me }) {
  if (!content || !me) return false;
  if (namesMentioned(content, me.names || [])) return true;
  if (mentionsEveryone(content) && memberCanMentionEveryone(server, authorId)) return true;
  const mine = (serverMember(server, me.id) || {}).roles || [];
  return roleMentions(content).some(id => mine.some(held => String(held).toLowerCase() === id)
    && roleMentionAllowed(server, authorId, roleById(server, id)));
}

function encodeRoleMentions(text, roles, reservedNames = []) {
  const reserved = new Set((reservedNames || []).filter(Boolean).map(n => n.toLowerCase()));
  const usable = (roles || []).filter(r => r.name && !reserved.has(r.name.toLowerCase()));
  if (!usable.length) return text;
  const byName = new Map(usable.map(r => [r.name.toLowerCase(), r.id]));
  const names = [...byName.keys()].sort((a, b) => b.length - a.length).map(escapeMentionName);
  const re = new RegExp(`(?<![\\p{L}\\p{N}_<])@(${names.join("|")})(?![\\p{L}\\p{N}_])`, "giu");
  return String(text || "").replace(re, (whole, name) => roleMentionToken(byName.get(name.toLowerCase())));
}

function decodeRoleMentions(text, roles) {
  return String(text || "").replace(ROLE_MENTION_RE, (whole, id) => {
    const role = roleById({ roles }, id);
    return role ? `@${role.name}` : UNKNOWN_ROLE_MENTION;
  });
}

function splitRoleMentions(text) {
  const parts = [];
  let last = 0;
  for (const match of String(text || "").matchAll(ROLE_MENTION_RE)) {
    if (match.index > last) parts.push({ text: text.slice(last, match.index) });
    parts.push({ role: match[1].toLowerCase() });
    last = match.index + match[0].length;
  }
  if (last < text.length) parts.push({ text: text.slice(last) });
  return parts;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    EVERYONE_MENTIONS, MENTION_EVERYONE_PERMISSION, UNKNOWN_ROLE_MENTION, namesMentioned, mentionsEveryone, isEveryoneMention,
    roleMentionToken, roleMentions, roleById, memberCanMentionEveryone, roleMentionAllowed, mentionableRoles,
    messageMentionsMe, encodeRoleMentions, decodeRoleMentions, splitRoleMentions,
  };
}
