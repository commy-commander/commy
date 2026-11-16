const HELP_SITE = "https://commy.chat/docs";
const HELP_PAGES = {
  accountServers: "account-servers",
  moving: "moving-your-account",
  syncServers: "sync-servers",
};
const SYNC_HOST_GUIDE_KEY = "commy.syncHostGuideDone";

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

function inviteHelpPage(invite) {
  return invite && invite.community_host && invite.community_host.ask ? "syncServers" : "accountServers";
}

function syncHostGuideSteps() {
  return [
    { title: "Get a sync server", text: "Run your own with Commy's sync server setup, or use one run by someone you trust." },
    { title: "Check it", text: "Type its address and choose Check and Move. Commy checks that it's ready and up to date before anything moves." },
    { title: "Move", text: "This device copies the server's setup and recent messages, up to 1,000 per channel. Messages stay sealed, so the sync server can't read them." },
    { title: "Members follow", text: "Members switch over the next time an up to date Commy opens. New members joining by invite are asked to trust a sync server that isn't on Commy's list." },
  ];
}

function syncHostGuideDone(storage) {
  try {
    return storage.getItem(SYNC_HOST_GUIDE_KEY) === "1";
  } catch {
    return false;
  }
}

function finishSyncHostGuide(storage, done) {
  try {
    if (done) storage.setItem(SYNC_HOST_GUIDE_KEY, "1");
    else storage.removeItem(SYNC_HOST_GUIDE_KEY);
  } catch {}
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    HELP_PAGES,
    helpUrl,
    shownHost,
    movePlan,
    movedInWelcome,
    inviteHelpPage,
    syncHostGuideSteps,
    syncHostGuideDone,
    finishSyncHostGuide,
  };
}
