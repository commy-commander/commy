const PROFILE_WASH = "linear-gradient(180deg, color-mix(in srgb, var(--pc) 46%, transparent), color-mix(in srgb, var(--pc) 12%, transparent))";
const PROFILE_FRIEND_KEYS = {
  none: { icon: "user-plus", title: "Add friend", cls: "key sq" },
  outgoing: { icon: "clock", title: "Request sent", cls: "key sq mc-sent" },
  incoming: { icon: "user-check", title: "Accept friend request", cls: "key sq primary mc-accept" },
};
const PROFILE_VIEW_KEY = { icon: "user", title: "View profile", cls: "key sq mc-view" };
const PROFILE_VIEW_PLACES = ["card", "dm"];
const CARD_GAP = 12;
const CARD_EDGE = 8;

function profileHasBanner(member) {
  return Boolean(member && ((typeof member.banner === "string" && member.banner.startsWith("blob:")) || member.role_banner));
}

function profileSubParts(member) {
  if (!member) return [];
  return [member.username, member.pronouns].map(part => String(part || "").trim()).filter(Boolean);
}

function profileStatusParts(member, back) {
  const emoji = String((member && member.status_emoji) || "").trim();
  const words = [back, member && member.custom_status].map(part => String(part || "").trim()).filter(Boolean).join(" · ");
  return { emoji, words, shown: Boolean(emoji || words) };
}

function profileFriendKey(friendState, { me = false, bot = false } = {}) {
  if (me || bot) return null;
  return PROFILE_FRIEND_KEYS[friendState] || null;
}

function profileMenuRows({ me = false, bot = false, friend = false, reportable = false, canTimeOut = false, canNickname = false } = {}) {
  if (me) return canNickname ? ["nickname"] : [];
  const rows = bot ? [] : ["invite"];
  if (friend) rows.push("notify", "unfriend");
  rows.push("block");
  if (reportable) rows.push("report");
  if (canTimeOut) rows.push("timeout");
  return rows;
}

function roleInk(color, studio) {
  if (!color) return "";
  if (!studio) return `#${color}`;
  return `color-mix(in oklab, #${color} var(--role-ink-mix, 100%), var(--header-primary))`;
}

function profileViewKey(place) {
  return PROFILE_VIEW_PLACES.includes(place) ? PROFILE_VIEW_KEY : null;
}

function memberCardLeft(anchor, cardWidth, viewWidth) {
  const before = anchor.left - cardWidth - CARD_GAP;
  if (before >= CARD_EDGE) return before;
  const after = (anchor.right ?? anchor.left) + CARD_GAP;
  return Math.max(CARD_EDGE, Math.min(after, viewWidth - cardWidth - CARD_EDGE));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { PROFILE_WASH, PROFILE_FRIEND_KEYS, profileHasBanner, profileSubParts, profileStatusParts, profileFriendKey, profileMenuRows, profileViewKey, memberCardLeft, roleInk };
}
