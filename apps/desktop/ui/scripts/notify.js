// Notification rules, kept free of DOM and backend so they can run in
// the app and in `node --test` alike (see ui/tests/notify.test.mjs).

// Every message notifies until you choose otherwise: a quiet first run
// hides that the app works at all.
const DEFAULT_NOTIFY_LEVEL = "all";

// Messages that catch up with us after a wake-up or a reconnect count
// as live when they are this fresh; older ones only bump unread counts,
// so opening the app after a day away is not a wall of toasts.
const RECENT_SECS = 15 * 60;

// Pure decision so the rules are easy to reason about: returns true
// when a message should raise a desktop notification. A post in an
// announcement channel counts as a mention: it is meant for everyone.
function shouldNotify({ level, presence, focusedOnIt, isDm, mentioned, announcement }) {
  if (presence === "do_not_disturb" || level === "none" || focusedOnIt) return false;
  if (isDm || level === "all") return true;
  // "online": everything, but only while your status is Online.
  if (level === "online") return presence === "online" || !presence;
  return level === "mentions" && Boolean(mentioned || announcement);
}

// Title and body of a notification for one live message. Announcements
// say so in the title, so they stand out from chat.
function messageNotice({ author_name, channel_name, server_name, content, has_attachments, announcement }, plain) {
  const body = plain(content || "") || (has_attachments ? "Sent an attachment" : "");
  if (announcement) {
    return { title: `Announcement in #${channel_name} (${server_name})`, body: `${author_name || "Someone"}: ${body}` };
  }
  return { title: `${author_name || "Someone"} (#${channel_name}, ${server_name})`, body };
}

// Whole names only, so "@Sam" does not ping "Samantha" and vice versa.
function mentionsName(content, names) {
  if (!content) return false;
  const escaped = [...names, "everyone", "here"].filter(Boolean)
    .map(n => n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  return new RegExp(`@(${escaped.join("|")})(?![\\p{L}\\p{N}_])`, "iu").test(content);
}

// Which notification level applies to a server, given the saved
// preferences: muted beats a per-server choice beats the global level.
function levelForServer(prefs, serverId) {
  if (prefs.muted && prefs.muted[serverId]) return "none";
  const own = prefs.servers && prefs.servers[serverId];
  if (own && own !== "default") return own;
  return prefs.level || DEFAULT_NOTIFY_LEVEL;
}

function isRecent(atSecs, nowSecs) {
  return Number.isFinite(atSecs) && nowSecs - atSecs <= RECENT_SECS;
}

// One notification for a channel's caught-up messages: the newest one,
// plus how many more came with it.
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
