const AUDIT_FILTERS = [
  ["all", "All actions"],
  ["server", "Server settings"],
  ["channels", "Channels"],
  ["roles", "Roles"],
  ["members", "Members"],
  ["messages", "Messages"],
  ["expressions", "Emoji, stickers and sounds"],
  ["events", "Events"],
  ["invites", "Invites"],
];

const AUDIT_GROUP_OF = {
  server_rename: "server",
  owner_transfer: "server",
  server_images: "server",
  server_public: "server",
  game_tags: "server",
  rules: "server",
  auto_mod: "server",
  afk_channel: "server",
  afk_timeout: "server",
  expressions: "expressions",
  sound_sharing: "expressions",
  channel_create: "channels",
  channel_delete: "channels",
  channel_rename: "channels",
  channel_retention: "channels",
  channel_move: "channels",
  channel_limit: "channels",
  channel_slow_mode: "channels",
  channel_banner: "channels",
  channel_color: "channels",
  channel_kind: "channels",
  role_create: "roles",
  role_update: "roles",
  role_delete: "roles",
  role_assign: "members",
  role_unassign: "members",
  member_kick: "members",
  member_ban: "members",
  member_unban: "members",
  voice_moderate: "members",
  member_timeout: "members",
  member_timeout_clear: "members",
  member_nickname: "members",
  emoji_add: "expressions",
  emoji_remove: "expressions",
  sound_add: "expressions",
  sound_remove: "expressions",
  event_create: "events",
  event_update: "events",
  event_delete: "events",
  message_pin: "messages",
  message_unpin: "messages",
  poll_close: "messages",
  message_delete: "messages",
  invite_create: "invites",
  invite_revoke: "invites",
  invite_background: "invites",
  sticker_add: "expressions",
  sticker_remove: "expressions",
  emoticon_add: "expressions",
  emoticon_remove: "expressions",
  expression_sharing: "expressions",
  channel_access: "channels",
};

const CHANNEL_KIND_WORDS = { text: "text channel", voice: "voice channel", announcement: "announcement channel", category: "category" };

const VOICE_MODERATION_WORDS = {
  muted: "server-muted",
  deafened: "server-deafened",
  "muted and deafened": "server-muted and deafened",
};

function auditGroup(kind) {
  return AUDIT_GROUP_OF[kind] || "server";
}

function auditStrong(text) {
  return { text, strong: true };
}

function auditCount(n, one, many) {
  return `${n} ${n === 1 ? one : many}`;
}

function describeAudit(entry) {
  const label = entry.label || null;
  const detail = entry.detail || null;
  const member = auditStrong(label || "a member");
  const channel = auditStrong(label || "a channel");
  const role = auditStrong(label || "a role");
  const named = fallback => auditStrong(label || fallback);
  const count = Number(detail);
  switch (entry.kind) {
    case "owner_transfer": return ["handed the server to ", member];
    case "server_rename": return label ? ["renamed the server from ", auditStrong(label), " to ", auditStrong(detail || "")] : ["renamed the server to ", auditStrong(detail || "")];
    case "server_images": return [`changed the server ${detail || "images"}`];
    case "server_public": return [detail === "on" ? "made the server public" : "made the server private"];
    case "game_tags": return detail ? ["set the server's games to ", auditStrong(detail)] : ["cleared the server's games"];
    case "rules": return [detail === "removed" ? "removed the server rules" : "updated the server rules"];
    case "auto_mod": return ["changed the auto moderation rules"];
    case "afk_channel": return label ? ["set the AFK channel to ", channel] : ["turned off the AFK channel"];
    case "afk_timeout": return detail ? [`set the server AFK timer to ${auditCount(count, "minute", "minutes")}`] : ["turned off the server AFK timer"];
    case "expressions": return ["changed who can share emoji and upload sounds"];
    case "sound_sharing": return [detail === "on" ? "let members play this server's sounds in their other servers" : "stopped sharing this server's sounds"];
    case "channel_create": return [`created the ${CHANNEL_KIND_WORDS[detail] || "channel"} `, channel];
    case "channel_delete": return label ? ["deleted the channel ", channel] : ["deleted a channel"];
    case "channel_rename": return ["renamed the channel ", channel, " to ", auditStrong(detail || "")];
    case "channel_retention": return detail ? ["set messages in ", channel, ` to disappear after ${auditCount(count, "day", "days")}`] : ["turned off disappearing messages in ", channel];
    case "channel_move": return ["moved the channel ", channel];
    case "channel_limit": return count > 0 ? [`set a limit of ${auditCount(count, "person", "people")} in `, channel] : ["removed the people limit in ", channel];
    case "channel_slow_mode": return count > 0 ? [`set slow mode to ${auditCount(count, "second", "seconds")} in `, channel] : ["turned off slow mode in ", channel];
    case "channel_banner": return [detail === "removed" ? "removed the banner of " : "changed the banner of ", channel];
    case "channel_color": return [detail ? "changed the color of " : "removed the color of ", channel];
    case "channel_kind": return ["turned ", channel, ` into a ${CHANNEL_KIND_WORDS[detail] || "channel"}`];
    case "channel_access": return detail === "private" ? ["made ", channel, " private"] : detail === "send" ? ["limited who can send in ", channel] : ["opened ", channel, " to everyone"];
    case "role_create": return ["created the role ", role];
    case "role_update": return ["updated the role ", role];
    case "role_delete": return label ? ["deleted the role ", role] : ["deleted a role"];
    case "role_assign": return ["gave ", member, " the ", auditStrong(detail || "a"), " role"];
    case "role_unassign": return ["took the ", auditStrong(detail || "a"), " role from ", member];
    case "member_kick": return ["kicked ", member];
    case "member_ban": return ["banned ", member];
    case "member_unban": return ["unbanned ", member];
    case "voice_moderate": return VOICE_MODERATION_WORDS[detail] ? [`${VOICE_MODERATION_WORDS[detail]} `, member] : ["lifted the server mute on ", member];
    case "member_timeout": return detail ? ["timed out ", member, ` for ${detail}`] : ["timed out ", member];
    case "member_timeout_clear": return ["removed the timeout on ", member];
    case "member_nickname": return detail ? ["changed ", member, "'s nickname to ", auditStrong(detail)] : ["cleared ", member, "'s nickname"];
    case "emoji_add": return ["added the emoji ", auditStrong(`:${label || "emoji"}:`)];
    case "emoji_remove": return ["removed the emoji ", auditStrong(`:${label || "emoji"}:`)];
    case "sticker_add": return ["added the sticker ", named("a sticker")];
    case "sticker_remove": return ["removed the sticker ", named("a sticker")];
    case "emoticon_add": return ["added the emoticon ", auditStrong(`:${label || "emoticon"}`)];
    case "emoticon_remove": return ["removed the emoticon ", auditStrong(`:${label || "emoticon"}`)];
    case "expression_sharing": return [detail ? `let members use this server's ${detail} in other servers and DMs` : "stopped sharing this server's stickers and emoticons"];
    case "sound_add": return ["added the sound ", named("a sound")];
    case "sound_remove": return ["removed the sound ", named("a sound")];
    case "event_create": return ["scheduled the event ", named("an event")];
    case "event_update": return ["updated the event ", named("an event")];
    case "event_delete": return ["cancelled the event ", named("an event")];
    case "message_pin": return ["pinned a message in ", channel];
    case "message_unpin": return ["unpinned a message in ", channel];
    case "poll_close": return ["closed a poll in ", channel];
    case "message_delete": return detail ? ["deleted a message by ", member, " in ", auditStrong(detail)] : ["deleted a message by ", member];
    case "invite_create": return ["created the invite ", named("a link")];
    case "invite_revoke": return detail ? ["revoked the invite ", named("a link"), " made by ", auditStrong(detail)] : ["revoked the invite ", named("a link")];
    case "invite_background": return [detail === "removed" ? "removed the invite background" : "changed the invite background"];
    default: return ["made a change"];
  }
}

function auditParts(entry) {
  return [auditStrong(entry.by_name || "Someone"), " ", ...describeAudit(entry)].map(part => (typeof part === "string" ? { text: part } : part));
}

function auditText(entry) {
  return auditParts(entry).map(part => part.text).join("");
}

function filterAudit(entries, { group = "all", by = "" } = {}) {
  return (entries || []).filter(e => (group === "all" || auditGroup(e.kind) === group) && (!by || e.by === by));
}

function auditActors(entries) {
  const seen = new Map();
  for (const e of entries || []) if (!seen.has(e.by)) seen.set(e.by, e.by_name || "Someone");
  return [...seen].map(([id, name]) => ({ id, name })).sort((a, b) => a.name.localeCompare(b.name));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { AUDIT_FILTERS, auditGroup, describeAudit, auditParts, auditText, filterAudit, auditActors };
}
