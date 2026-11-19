const SYNC_HOST_MAX_CHARS = 200;
const SYNC_HOST_HOME = "Commy.chat";
const SYNC_HOST_INVALID = "Use the address of a Commy sync server on a public domain, like sync.yourcommunity.org";

function syncHostOk(host) {
  const check = typeof storeHostOk === "function" ? storeHostOk : require("./file-store.js").storeHostOk;
  return check(host);
}

function cleanSyncHost(input) {
  const text = String(input || "").trim();
  if (!text || text.length > SYNC_HOST_MAX_CHARS) return null;
  const scheme = text.match(/^([a-z][a-z0-9+.-]*):\/\//i);
  if (scheme && !["https", "wss"].includes(scheme[1].toLowerCase())) return null;
  const rest = (scheme ? text.slice(scheme[0].length) : text).replace(/\/+$/, "");
  const slash = rest.indexOf("/");
  const host = slash >= 0 ? rest.slice(0, slash) : rest;
  const path = slash >= 0 ? rest.slice(slash + 1).replace(/\/+$/, "") : null;
  if (path !== null && path !== "sync") return null;
  return syncHostOk(host) ? host.toLowerCase() : null;
}

function syncHostHomeLabel(ownerHome) {
  const host = String(ownerHome || "").trim().toLowerCase();
  return !host || host === "commy.chat" ? SYNC_HOST_HOME : host;
}

function syncHostHomeButton(ownerHome) {
  return `Move back to ${syncHostHomeLabel(ownerHome)}`;
}

function syncHostPlace(host, release) {
  const operator = release && release.operator;
  return operator && operator.toLowerCase() !== host.toLowerCase() ? `${operator} at ${host}` : host;
}

function syncHostIsOfficial(host) {
  return host === SYNC_HOST_HOME || String(host).toLowerCase() === "commy.chat";
}

function syncHostNow(server, release, ownerHome) {
  const host = (server && server.sync_host) || syncHostHomeLabel(ownerHome);
  if (syncHostIsOfficial(host)) {
    const now = "Right now this server syncs through Commy.chat, Commy's official sync server.";
    const notice = syncHostAppNotice(release);
    return notice ? `${now} ${notice}` : now;
  }
  const now = `Right now this server syncs through ${syncHostPlace(host, release)}.`;
  return release && release.text ? `${now} ${release.text}` : now;
}

function syncHostReady(label, release, home) {
  const runs = release && release.text ? ` ${release.text}` : "";
  const next = home ? `Choose ${syncHostHomeButton(label)} again to move it back.` : "Choose Move Now to move this server there.";
  return `${label} is ready.${runs} ${next}`;
}

function syncHostAppNotice(release) {
  return release && release.fit === "app_behind" ? release.text : null;
}

if (typeof module !== "undefined") {
  module.exports = { SYNC_HOST_MAX_CHARS, SYNC_HOST_HOME, SYNC_HOST_INVALID, cleanSyncHost, syncHostHomeLabel, syncHostHomeButton, syncHostNow, syncHostReady, syncHostAppNotice };
}
