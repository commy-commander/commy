"use strict";

const PERMISSION_HINTS = {
  get manage_server() { return t("ui.rename-the-server-and-change-its-images"); },
  get manage_channels() { return t("ui.create-rename-reorder-and-delete"); },
  get manage_roles() { return t("ui.create-roles-change-what-they-allow-and"); },
  get manage_expressions() { return t("ui.replace-or-remove-any-emoji-sticker"); },
  get create_events() { return t("ui.schedule-events-and-edit-or-delete-the"); },
  get manage_events() { return t("ui.edit-or-delete-any-scheduled-event"); },
  get view_audit_log() { return t("ui.read-who-changed-what-in-the-server"); },
  get manage_file_storage() { return t("ui.turn-file-backup-on-or-off-and-keep-its"); },
  get manage_messages() { return t("ui.delete-or-pin-anyone-s-messages-and-tend"); },
  get pin_messages() { return t("ui.pin-and-unpin-messages-in-channels-they"); },
  get attach_files() { return t("ui.upload-files-and-images-and-paste"); },
  get send_voice_messages() { return t("ui.record-voice-messages-from-the-composer"); },
  get send_tts_messages() { return t("ui.send-tts-messages-that-readers-can-have"); },
  get embed_links() { return t("ui.links-they-send-show-a-preview-card-and"); },
  get read_message_history() { return t("ui.see-what-was-said-before-they-opened-a"); },
  get create_threads() { return t("ui.name-archive-or-lock-a-thread-under"); },
  get send_polls() { return t("ui.post-polls-from-the-composer-menu"); },
  get post_announcements() { return t("ui.write-in-announcement-channels"); },
  get mention_everyone() { return t("ui.notify-everyone-here-or-any-role-at-once"); },
  get bypass_slow_mode() { return t("ui.send-without-waiting-when-a-channel-is"); },
  get create_invites() { return t("ui.make-invite-links-and-codes-for-this"); },
  get add_emojis() { return t("ui.add-new-custom-emojis-to-the-server"); },
  get add_stickers() { return t("ui.add-new-stickers-to-the-server"); },
  get add_emoticons() { return t("ui.save-new-text-emoticons-to-the-server"); },
  get use_external_emojis() { return t("ui.use-emojis-and-stickers-shared-by-other"); },
  get kick_members() { return t("ui.remove-members-who-can-come-back-with-an"); },
  get ban_members() { return t("ui.ban-and-unban-members"); },
  get mute_members() { return t("ui.mute-and-deafen-people-in-voice-for"); },
  get time_out_members() { return t("ui.stop-someone-from-sending-or-reacting"); },
  get move_members() { return t("ui.move-people-between-voice-channels"); },
  get manage_nicknames() { return t("ui.change-other-members-nicknames-in-this"); },
  get change_nickname() { return t("ui.pick-their-own-nickname-in-this-server"); },
  get priority_speaker() { return t("ui.turn-everyone-else-down-while-they-talk"); },
  get use_voice_filters() { return t("ui.change-how-they-sound-in-voice-with-a"); },
  get administrator() { return t("ui.everything-and-nothing-can-be-refused"); },
};

const EVERYONE_CHAT_ACCESS = [
  { key: "embeds", permission: "embed_links", field: "embed_links", get title() { return t("ui.embed-links"); }, get sub() { return t("ui.links-members-send-show-a-preview-card"); } },
  { key: "history", permission: "read_message_history", field: "read_history", get title() { return t("ui.read-message-history"); }, get sub() { return t("ui.members-see-what-was-said-before-they"); } },
  { key: "threads", permission: "create_threads", field: "create_threads", get title() { return t("ui.start-threads"); }, get sub() { return t("ui.members-name-archive-or-lock-threads"); } },
  { key: "polls", permission: "send_polls", field: "send_polls", get title() { return t("ui.create-polls"); }, get sub() { return t("ui.members-post-polls-from-the-composer"); } },
  { key: "events", permission: "create_events", field: "create_events", get title() { return t("ui.create-events"); }, get sub() { return t("ui.members-schedule-events-and-edit-or"); } },
  { key: "voice_filters", permission: "use_voice_filters", field: "voice_filters", get title() { return t("ui.use-voice-filters"); }, get sub() { return t("ui.members-change-how-they-sound-in-voice"); } },
];

const RESTRICTION_NAMES = ["embed_links", "read_message_history", "create_threads", "send_polls", "create_events", "use_voice_filters"];
const HISTORY_NOTICE = "You can see new messages only";
const LINK_EMBED_OFF = { get title() { return t("ui.link-previews-are-off-for-you-here"); }, get sub() { return t("ui.your-link-posts-as-plain-text"); } };

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
