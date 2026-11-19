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
    title: t("ui.move-your-account-to", { host }),
    comes: t("ui.your-identity-profile-friends-server"),
    steps: [
      t("ui.this-device-copies-your-account-to-then", { host }),
      t("ui.you-sign-in-or-create-an-account-on", { host }),
      t("ui.commy-brings-back-your-servers-and"),
    ],
    others: `People you talk to don't need to do anything. Their apps find you on ${host}, and people on other account servers see "Signed in through ${host}" by your name.`,
    note: t("ui.your-old-account-server-keeps-a-short", { host }),
  };
}

function movedInWelcome(moved) {
  const from = shownHost(moved && moved.from);
  const to = shownHost(moved && moved.to);
  return {
    title: t("ui.welcome-to-4", { to }),
    lead: t("ui.your-account-moved-here-from-your", { from }),
    points: [
      t("ui.your-other-devices-show-your-account"),
      t("ui.people-on-other-account-servers-see", { to }),
      t("ui.keeps-a-note-pointing-here-for-a-year-or", { from }),
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
