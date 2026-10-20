const DEFAULT_NOTIFY_LEVEL = "all";

const RECENT_SECS = 15 * 60;

function shouldNotify({ level, presence, focusedOnIt, isDm, mentioned, announcement }) {
  if (presence === "do_not_disturb" || level === "none" || focusedOnIt) return false;
  if (isDm || level === "all") return true;
  if (level === "online") return presence === "online" || !presence;
  return level === "mentions" && Boolean(mentioned || announcement);
}

function noticeText(content, hasAttachments, plain, voiceMessage) {
  const text = plain(content || "");
  if (text) return text;
  if (voiceMessage) return "Voice message";
  return hasAttachments ? "Sent an attachment" : "";
}

function noticeTitle(authorName, channelName, serverName) {
  return `${authorName || "Someone"} (#${channelName}, ${serverName})`;
}

function messageNotice({ author_name, channel_name, server_name, content, has_attachments, announcement, voice_message }, plain) {
  const body = noticeText(content, has_attachments, plain, voice_message);
  if (announcement) {
    return { title: `Announcement in #${channel_name} (${server_name})`, body: `${author_name || "Someone"}: ${body}` };
  }
  return { title: noticeTitle(author_name, channel_name, server_name), body };
}

function mentionsName(content, names) {
  if (!content) return false;
  const escaped = names.filter(Boolean).map(n => n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  if (!escaped.length) return false;
  return new RegExp(`@(${escaped.join("|")})(?![\\p{L}\\p{N}_])`, "iu").test(content);
}

function levelForServer(prefs, serverId) {
  if (prefs.muted && prefs.muted[serverId]) return "none";
  const own = prefs.servers && prefs.servers[serverId];
  if (own && own !== "default") return own;
  return prefs.level || DEFAULT_NOTIFY_LEVEL;
}

const MUTE_CHOICES = [
  { id: "15m", label: "For 15 minutes", seconds: 15 * 60 },
  { id: "1h", label: "For 1 hour", seconds: 60 * 60 },
  { id: "8h", label: "For 8 hours", seconds: 8 * 60 * 60 },
  { id: "24h", label: "For 24 hours", seconds: 24 * 60 * 60 },
  { id: "forever", label: "Until I turn it back on", seconds: null },
];

function channelPref(prefs, serverId, channelId) {
  const own = prefs.channels && prefs.channels[serverId];
  return (own && own[channelId]) || null;
}

function isMuted(prefs, serverId, channelId, nowSecs) {
  if (prefs.muted && prefs.muted[serverId]) return true;
  const own = channelPref(prefs, serverId, channelId);
  if (!own) return false;
  if (own.mute === true) return true;
  return typeof own.mute === "number" && own.mute > nowSecs;
}

function levelForChannel(prefs, serverId, channelId, nowSecs) {
  if (isMuted(prefs, serverId, channelId, nowSecs)) return "none";
  const own = channelPref(prefs, serverId, channelId);
  if (own && own.level && own.level !== "default") return own.level;
  return levelForServer(prefs, serverId);
}

function withChannelPref(prefs, serverId, channelId, change) {
  const channels = { ...(prefs.channels || {}) };
  const own = { ...(channelPref(prefs, serverId, channelId) || {}) };
  if ("level" in change) {
    if (!change.level || change.level === "default") delete own.level;
    else own.level = change.level;
  }
  if ("mute" in change) {
    if (change.mute === true || typeof change.mute === "number") own.mute = change.mute;
    else delete own.mute;
  }
  const forServer = { ...(channels[serverId] || {}) };
  if (Object.keys(own).length) forServer[channelId] = own;
  else delete forServer[channelId];
  if (Object.keys(forServer).length) channels[serverId] = forServer;
  else delete channels[serverId];
  return { ...prefs, channels };
}

function muteUntil(choice, nowSecs) {
  return choice.seconds === null ? true : nowSecs + choice.seconds;
}

function isRecent(atSecs, nowSecs) {
  return Number.isFinite(atSecs) && nowSecs - atSecs <= RECENT_SECS;
}

function caughtUpNotice({ count, author_name, channel_name, server_name, content, has_attachments, voice_message }, plain) {
  const text = noticeText(content, has_attachments, plain, voice_message);
  const rest = count - 1;
  const more = rest > 0 ? `${rest} more message${rest === 1 ? "" : "s"}` : "";
  return {
    title: noticeTitle(author_name, channel_name, server_name),
    body: [text, more].filter(Boolean).join(" · "),
  };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    DEFAULT_NOTIFY_LEVEL, RECENT_SECS, MUTE_CHOICES, shouldNotify, noticeText, messageNotice, mentionsName, levelForServer, levelForChannel, isMuted,
    channelPref, withChannelPref, muteUntil, isRecent, caughtUpNotice,
  };
}
