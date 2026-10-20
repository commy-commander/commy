"use strict";

const PERMISSION_HINTS = {
  manage_server: "Rename the server and change its images, rules, onboarding and auto moderation.",
  manage_channels: "Create, rename, reorder and delete channels and categories.",
  manage_roles: "Create roles, change what they allow and hand them out.",
  manage_expressions: "Replace or remove any emoji, sticker, emoticon or soundboard clip.",
  create_events: "Schedule events, and edit or delete the ones they created.",
  manage_events: "Edit or delete any scheduled event.",
  view_audit_log: "Read who changed what in the server.",
  manage_messages: "Delete or pin anyone's messages and tend any thread or poll.",
  pin_messages: "Pin and unpin messages in channels they can see.",
  attach_files: "Upload files and images and paste pictures into chat.",
  send_voice_messages: "Record voice messages from the composer.",
  send_tts_messages: "Send /tts messages that readers can have read aloud.",
  embed_links: "Links they send show a preview card and inline media.",
  read_message_history: "See what was said before they opened a channel.",
  create_threads: "Name, archive or lock a thread under their own message.",
  send_polls: "Post polls from the composer menu.",
  post_announcements: "Write in announcement channels.",
  mention_everyone: "Notify everyone, here, or any role at once.",
  bypass_slow_mode: "Send without waiting when a channel is in slow mode.",
  create_invites: "Make invite links and codes for this server.",
  add_emojis: "Add new custom emojis to the server.",
  add_stickers: "Add new stickers to the server.",
  add_emoticons: "Save new text emoticons to the server.",
  use_external_emojis: "Use emojis and stickers shared by other servers.",
  kick_members: "Remove members, who can come back with an invite.",
  ban_members: "Ban and unban members.",
  mute_members: "Mute and deafen people in voice for everyone.",
  time_out_members: "Stop someone from sending or reacting for a while.",
  move_members: "Move people between voice channels.",
  manage_nicknames: "Change other members' nicknames in this server.",
  change_nickname: "Pick their own nickname in this server.",
  priority_speaker: "Turn everyone else down while they talk.",
  administrator: "Everything, and nothing can be refused.",
};

const EVERYONE_CHAT_ACCESS = [
  { key: "embeds", permission: "embed_links", field: "embed_links", title: "Embed links", sub: "Links members send show a preview card and inline images." },
  { key: "history", permission: "read_message_history", field: "read_history", title: "Read message history", sub: "Members see what was said before they opened a channel." },
  { key: "threads", permission: "create_threads", field: "create_threads", title: "Start threads", sub: "Members name, archive or lock threads under their own messages." },
  { key: "polls", permission: "send_polls", field: "send_polls", title: "Create polls", sub: "Members post polls from the composer menu." },
  { key: "events", permission: "create_events", field: "create_events", title: "Create events", sub: "Members schedule events and edit or delete their own." },
];

const RESTRICTION_NAMES = ["embed_links", "read_message_history", "create_threads", "send_polls", "create_events"];
const HISTORY_NOTICE = "You can see new messages only";
const LINK_EMBED_OFF = { title: "Link previews are off for you here", sub: "Your link posts as plain text" };

function permissionHint(name) {
  return PERMISSION_HINTS[name] || "";
}

function isRestriction(name) {
  return RESTRICTION_NAMES.includes(name);
}

function switchOn(heldNames, name) {
  const held = heldNames instanceof Set ? heldNames.has(name) : (heldNames || []).includes(name);
  return isRestriction(name) ? !held : held;
}

function namesToSave(switches) {
  return switches.filter(([name, on]) => (isRestriction(name) ? !on : on)).map(([name]) => name);
}

function grantedNames(heldNames) {
  return (heldNames || []).filter(name => !isRestriction(name));
}

function everyoneLacks(server, spec) {
  const restricted = server && server.chat_restricted;
  return Boolean(restricted && restricted[spec.field]);
}

function canEmbedLinks(server) {
  return !server || server.can_embed_links !== false;
}

function canReadHistory(server) {
  return !server || server.can_read_history !== false;
}

function canCreateThreads(server) {
  return !server || server.can_create_threads !== false;
}

function canSendPolls(server) {
  return !server || server.can_send_polls !== false;
}

function canCreateEvents(server) {
  if (!server) return true;
  return server.can_create_events !== false || Boolean(server.can_manage_events);
}

function showsEmbeds(message) {
  return !message || message.embeds !== false;
}

function threadEntryShown({ replies, meta, canCreate } = {}) {
  return Boolean(meta) || (Number(replies) || 0) > 0 || canCreate !== false;
}

function historyWindow(messages, openedAt) {
  if (openedAt === null || openedAt === undefined) return [];
  const from = Number(openedAt);
  if (!Number.isFinite(from)) return [];
  return (messages || []).filter(m => Date.parse(m.created_at) >= from);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    PERMISSION_HINTS, EVERYONE_CHAT_ACCESS, RESTRICTION_NAMES, HISTORY_NOTICE, LINK_EMBED_OFF,
    permissionHint, isRestriction, switchOn, namesToSave, grantedNames, everyoneLacks, canEmbedLinks, canReadHistory, canCreateThreads, canSendPolls, canCreateEvents,
    showsEmbeds, threadEntryShown, historyWindow,
  };
}
