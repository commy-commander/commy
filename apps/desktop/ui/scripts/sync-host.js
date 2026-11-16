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

function syncHostNow(server, release, ownerHome = SYNC_HOST_HOME) {
  const now = `This server syncs through ${(server && server.sync_host) || ownerHome || SYNC_HOST_HOME}.`;
  return release && release.text ? `${now} ${release.text}` : now;
}

function syncHostReady(label, release, home) {
  const runs = release && release.text ? ` ${release.text}` : "";
  return `${label} is ready.${runs} Choose ${home ? "Move back to Commy.chat" : "Move Now"} again to move this server there.`;
}

function syncHostAppNotice(release) {
  return release && release.fit === "app_behind" ? release.text : null;
}

if (typeof module !== "undefined") {
  module.exports = { SYNC_HOST_MAX_CHARS, SYNC_HOST_HOME, SYNC_HOST_INVALID, cleanSyncHost, syncHostNow, syncHostReady, syncHostAppNotice };
}
