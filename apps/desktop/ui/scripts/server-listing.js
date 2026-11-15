const TRUST_AND_JOIN = "Trust and Join";

function inviteAsksTrust(invite) {
  return Boolean(invite && invite.community_host && invite.community_host.ask);
}

function communityHostWarning(host) {
  return `This community keeps its messages on ${host}, which is not on Commy's list of servers. ` +
    `${host} cannot read the messages, but it can see when members connect. Only join if you trust it.`;
}

function accountServerWarning(invite) {
  return `This invite comes from ${invite.host}, not your account server (${invite.account_server_host}). ` +
    "Anyone can run a Commy server, so only join if you trust the person who sent this link. " +
    "Your account stays on your own server either way.";
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
  if (choice === "default") return "signin.server.note";
  const homes = (account && account.listed_homes) || [];
  const listed = homes.some(home => home.origin === choice) || (choice === account?.server && account?.server_listed);
  return listed ? "signin.server.listed" : "signin.server.warning";
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { TRUST_AND_JOIN, inviteAsksTrust, inviteWarningText, communityHostWarning, listedHomeLabel, listedHomeOptions, signInServerNote };
}
