const DEFAULT_NOTIFY_LEVEL = "all";

const RECENT_SECS = 15 * 60;

function shouldNotify({ level, presence, focusedOnIt, isDm, mentioned, announcement }) {
  if (presence === "do_not_disturb" || level === "none" || focusedOnIt) return false;
  if (isDm || level === "all") return true;
  if (level === "online") return presence === "online" || !presence;
  return level === "mentions" && Boolean(mentioned || announcement);
}

function messageNotice({ author_name, channel_name, server_name, content, has_attachments, announcement }, plain) {
  const body = plain(content || "") || (has_attachments ? "Sent an attachment" : "");
  if (announcement) {
    return { title: `Announcement in #${channel_name} (${server_name})`, body: `${author_name || "Someone"}: ${body}` };
  }
  return { title: `${author_name || "Someone"} (#${channel_name}, ${server_name})`, body };
}

function mentionsName(content, names) {
  if (!content) return false;
  const escaped = [...names, "everyone", "here"].filter(Boolean)
    .map(n => n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  return new RegExp(`@(${escaped.join("|")})(?![\\p{L}\\p{N}_])`, "iu").test(content);
}

function levelForServer(prefs, serverId) {
  if (prefs.muted && prefs.muted[serverId]) return "none";
  const own = prefs.servers && prefs.servers[serverId];
  if (own && own !== "default") return own;
  return prefs.level || DEFAULT_NOTIFY_LEVEL;
}

function isRecent(atSecs, nowSecs) {
  return Number.isFinite(atSecs) && nowSecs - atSecs <= RECENT_SECS;
}

function caughtUpNotice({ count, author_name, channel_name, server_name, content, has_attachments }, plain) {
  const text = plain(content || "") || (has_attachments ? "Sent an attachment" : "");
  const rest = count - 1;
  const more = rest > 0 ? `${rest} more message${rest === 1 ? "" : "s"}` : "";
  return {
    title: `${author_name || "Someone"} (#${channel_name}, ${server_name})`,
    body: [text, more].filter(Boolean).join(" · "),
  };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { DEFAULT_NOTIFY_LEVEL, RECENT_SECS, shouldNotify, messageNotice, mentionsName, levelForServer, isRecent, caughtUpNotice };
}
