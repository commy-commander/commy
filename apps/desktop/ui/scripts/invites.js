const INVITE_KEY = "[A-Za-z0-9_-]";
const INVITE_LINK_RE = new RegExp(
  "\\b(?:https?:\\/\\/[a-z0-9.\\-:[\\]]+\\/i\\/|commy:\\/\\/i\\/[^\\s/]+\\/)" +
  `(?:[a-hj-np-z2-9]{8}(?:#${INVITE_KEY}{43}(?!${INVITE_KEY})|\\b\\/?)` +
  `|[a-z0-9]+(?:-[a-z0-9]+)*#(?:${INVITE_KEY}{43}|${INVITE_KEY}{22})(?!${INVITE_KEY}))`,
  "i",
);

const INVITE_NAME_MIN = 3;
const INVITE_NAME_MAX = 32;
const RESERVED_INVITE_NAMES = new Set([
  "admin", "commy", "support", "help", "staff", "official", "moderator", "mod", "team", "security", "login",
  "account", "download", "docs", "privacy", "invite", "invites",
]);

function findInviteLink(text) {
  const m = INVITE_LINK_RE.exec(text || "");
  return m ? m[0] : null;
}

function looksLikeInviteLink(text) {
  const link = findInviteLink(text);
  return Boolean(link) && link === String(text).trim();
}

function checkInviteName(input) {
  const name = String(input || "").trim().toLowerCase();
  if (!name) return { name: "", error: null };
  if (name.length === 8 && !name.includes("-")) {
    return { name, error: "Eight characters with no dash looks like a random invite code. Add a dash or pick another length." };
  }
  const shape = name.length >= INVITE_NAME_MIN && name.length <= INVITE_NAME_MAX &&
    /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(name);
  if (!shape) return { name, error: "Use 3 to 32 letters, numbers or single dashes, like pixel-pioneers." };
  if (RESERVED_INVITE_NAMES.has(name)) return { name, error: "That name is reserved. Pick another one." };
  return { name, error: null };
}

function inviteTimeLeft(expiresAt, now = Date.now()) {
  if (expiresAt == null) return "Never expires";
  const left = expiresAt - now;
  if (left <= 0) return "Expired";
  const minutes = Math.floor(left / 60e3);
  if (minutes < 1) return "Less than a minute left";
  const unit = (n, one) => `${n} ${one}${n === 1 ? "" : "s"} left`;
  if (minutes < 60) return unit(minutes, "minute");
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return minutes % 60 && hours < 6 ? `${hours}h ${minutes % 60}m left` : unit(hours, "hour");
  return unit(Math.floor(hours / 24), "day");
}

function inviteLabel(invite) {
  return `${invite.host}/i/${invite.code}`;
}

function sortInvites(invites) {
  return [...(invites || [])].sort((a, b) => (b.created_at - a.created_at) || inviteLabel(a).localeCompare(inviteLabel(b)));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { INVITE_LINK_RE, findInviteLink, looksLikeInviteLink, checkInviteName, inviteTimeLeft, inviteLabel, sortInvites };
}
