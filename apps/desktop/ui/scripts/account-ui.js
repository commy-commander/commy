const UI_OWNER_KEY = "commy.uiOwner";
const ACCOUNT_UI_KEYS = [
  "commy.pins.seen",
  "commy.switcher.recent",
  "commy.hiddenMessages",
  "commy.collapsedSections",
  "commy.sensitiveConfirmed",
  "commy.cameraBackdrop",
];

function claimUiState(storage, accountId) {
  if (!storage || !accountId) return 0;
  if (storage.getItem(UI_OWNER_KEY) === accountId) return 0;
  let forgotten = 0;
  for (const key of ACCOUNT_UI_KEYS) {
    if (storage.getItem(key) === null) continue;
    storage.removeItem(key);
    forgotten++;
  }
  storage.setItem(UI_OWNER_KEY, accountId);
  return forgotten;
}

function unclaimedProfileLine(profile, t) {
  if (!profile) return "";
  const name = profile.display_name || profile.username || t("claim.unnamed");
  const handle = profile.username && profile.display_name ? ` (@${profile.username})` : "";
  const servers = t(profile.servers === 1 ? "claim.servers.one" : "claim.servers.many", { count: profile.servers || 0 });
  return `${name}${handle} · ${servers}`;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { UI_OWNER_KEY, ACCOUNT_UI_KEYS, claimUiState, unclaimedProfileLine };
}
