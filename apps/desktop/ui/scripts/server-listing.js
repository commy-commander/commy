const TRUST_AND_JOIN = "Trust and Join";

function inviteAsksTrust(invite) {
  return Boolean(invite && invite.community_host && invite.community_host.ask);
}

function communityHostWarning(host) {
  return t("ui.this-community-keeps-its-messages-on", { host }) +
    t("ui.cannot-read-the-messages-but-it-can-see", { host });
}

function accountServerWarning(invite) {
  return t("ui.this-invite-comes-from-not-your-account", { host: invite.host, account_server_host: invite.account_server_host }) +
    t("ui.anyone-can-run-a-commy-server-so-only") +
    t("ui.your-account-stays-on-your-own-server");
}

function inviteWarningText(invite) {
  const parts = [];
  if (!invite.trusted) parts.push(accountServerWarning(invite));
  if (inviteAsksTrust(invite)) parts.push(communityHostWarning(invite.community_host.host));
  return parts.join(" ");
}

function listedHomeLabel(home) {
  return home.name && home.name !== home.host ? `${home.name} (${home.host})` : home.host;
}

function listedHomeOptions(homes, chosen) {
  return (homes || []).map(home => ({ value: home.origin, label: listedHomeLabel(home), selected: home.origin === chosen }));
}

function signInServerNote(choice, account) {
  if (choice === "default") return account?.server_impostor ? "signin.server.unverified" : "signin.server.note";
  const homes = (account && account.listed_homes) || [];
  const listed = homes.some(home => home.origin === choice) || (choice === account?.server && account?.server_listed);
  return listed ? "signin.server.listed" : "signin.server.warning";
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { TRUST_AND_JOIN, inviteAsksTrust, inviteWarningText, communityHostWarning, listedHomeLabel, listedHomeOptions, signInServerNote };
}
