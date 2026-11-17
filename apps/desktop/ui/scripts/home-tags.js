const HOME_TAG_TEXT = {
  verified: "Signed in through {home}",
  checking: "Signed in through {home} (checking)",
  unverified: "Says it signs in through {home}, but that server does not vouch for this person",
  lookalike: "{home} looks like another server's address. Be careful who you trust.",
  blocked: "{home} is on Commy's list of blocked servers",
  expired: "This person's home card for {home} has run out",
};
const MAX_BLOCKED_HOMES = 200;
const BLOCKED_HOME_HIDDEN = "Hidden because this person's server is blocked by Commy";
const HOST_BLOCKED_TEXT = "This server syncs through {host}, which is on Commy's list of blocked servers, so it no longer syncs.";

function hostBlockedText(host) {
  return HOST_BLOCKED_TEXT.replace("{host}", host || "a server");
}

function homeTagState(tag, status) {
  if (!tag) return null;
  if (status && status.blocked) return "blocked";
  if (tag.lookalike) return "lookalike";
  if (!tag.current) return "expired";
  if (!status || !status.checked) return "checking";
  return status.key === tag.key ? "verified" : "unverified";
}

function homeTagTitle(tag, state) {
  return (HOME_TAG_TEXT[state] || HOME_TAG_TEXT.checking).replace("{home}", tag.shown);
}

function homeTagWarns(state) {
  return state === "lookalike" || state === "unverified" || state === "blocked" || state === "expired";
}

function showsHomeInChat(tag, myHome, state) {
  if (!tag) return false;
  return tag.home !== myHome || homeTagWarns(state);
}

function homesToAsk(members, known) {
  const homes = new Set();
  for (const member of members || []) {
    const home = member && member.home && member.home.home;
    if (home && !(known && known.has(home))) homes.add(home);
  }
  return [...homes].sort();
}

function cleanHomeInput(raw) {
  const text = String(raw || "").trim().toLowerCase().replace(/^https?:\/\//, "").replace(/\/.*$/, "").replace(/\.$/, "");
  const label = "[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?";
  return new RegExp(`^(${label}\\.)+[a-z][a-z0-9-]{0,61}[a-z0-9]$`).test(text) && text.length <= 253 ? text : null;
}

function withBlockedHome(homes, raw) {
  const home = cleanHomeInput(raw);
  if (!home) return { homes, error: "Type a server address like example.org" };
  if ((homes || []).includes(home)) return { homes, error: null };
  if ((homes || []).length >= MAX_BLOCKED_HOMES) return { homes, error: `You can block up to ${MAX_BLOCKED_HOMES} servers` };
  return { homes: [...(homes || []), home].sort(), error: null };
}

function withoutBlockedHome(homes, home) {
  return (homes || []).filter(h => h !== home);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    HOME_TAG_TEXT, MAX_BLOCKED_HOMES, BLOCKED_HOME_HIDDEN, HOST_BLOCKED_TEXT,
    hostBlockedText, homeTagState, homeTagTitle, homeTagWarns, showsHomeInChat, homesToAsk, cleanHomeInput, withBlockedHome, withoutBlockedHome,
  };
}
