const HELP_SITE = "https://commy.chat/docs";
const HELP_PAGES = {
  accountServers: "account-servers",
  moving: "moving-your-account",
  syncServers: "sync-servers",
  choosing: "choosing-a-server",
};

function helpUrl(page) {
  const slug = HELP_PAGES[page];
  return slug ? `${HELP_SITE}/${slug}` : HELP_SITE;
}

function shownHost(origin) {
  return String(origin || "").trim().replace(/^https:\/\//i, "").replace(/\/+$/, "");
}

function movePlan(to) {
  const host = shownHost(to);
  return {
    title: `Move your account to ${host}?`,
    comes: "Your identity, profile, friends, server list and settings come with you. They stay sealed, so the new account server can't read them. Your servers stay where they live and keep working for everyone in them.",
    steps: [
      `This device copies your account to ${host}, then signs out here.`,
      `You sign in or create an account on ${host}.`,
      "Commy brings back your servers and settings on its own.",
    ],
    others: `People you talk to don't need to do anything. Their apps find you on ${host}, and people on other account servers see "Signed in through ${host}" by your name.`,
    note: `Your old account server keeps a short public note that points to ${host}, so your other devices and contacts can follow. It goes away after a year, or sooner if you delete your account there.`,
  };
}

function movedInWelcome(moved) {
  const from = shownHost(moved && moved.from);
  const to = shownHost(moved && moved.to);
  return {
    title: `Welcome to ${to}`,
    lead: `Your account moved here from ${from}. Your identity, profile, friends and settings came with you, and your servers are coming back now.`,
    points: [
      `Your other devices show "Your account moved" in Settings, Commy Account. Choose Sign In There on each one.`,
      `People on other account servers see "Signed in through ${to}" by your name.`,
      `${from} keeps a note pointing here for a year, or until you delete your account there.`,
    ],
  };
}

function signInHelp(noteKey) {
  return noteKey === "signin.server.warning"
    ? { page: "choosing", key: "signin.server.trust" }
    : { page: "accountServers", key: "signin.server.help" };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    HELP_PAGES,
    helpUrl,
    shownHost,
    movePlan,
    movedInWelcome,
    signInHelp,
  };
}
