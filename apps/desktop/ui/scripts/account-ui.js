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

function accountRows(account) {
  if (!account || !account.signed_in || !account.user || !Array.isArray(account.others)) return [];
  const current = { ...account.user, server: account.server, server_host: account.server_host, current: true };
  return [current, ...account.others.map(other => ({ ...other, current: false }))];
}

function canAddAccount(account) {
  const rows = accountRows(account);
  return rows.length > 0 && rows.length < (account.max_accounts || 0);
}

function accountLine(row, account) {
  return row.server === account.server ? row.email : `${row.email} · ${row.server_host}`;
}

function signOutPrompt(account, t) {
  const next = accountRows(account).find(row => !row.current);
  return next ? t("accounts.signOut.next", { name: next.name || next.email }) : t("accounts.signOut.last");
}

function accountChangeStep(was, account, addingAccount) {
  if (account.signed_in && !was) return "open";
  if (!account.signed_in && (was || account.pending_code)) return "sign-in";
  if (account.signed_in && addingAccount) return "accounts";
  return null;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { UI_OWNER_KEY, ACCOUNT_UI_KEYS, claimUiState, unclaimedProfileLine, accountRows, canAddAccount, accountLine, signOutPrompt, accountChangeStep };
}
