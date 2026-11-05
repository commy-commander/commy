const FILE_STORE_MAX_CHARS = 200;
const FILE_STORE_GUIDE = "https://github.com/commy-commander/commy/tree/main/apps/file-store";
const FILE_STORE_INVALID = "Use the https address of your file storage, like https://commy-files.yourname.workers.dev";

function privateStoreHost(host) {
  return host === "localhost"
    || /\.(localhost|local|internal|lan)$/.test(host)
    || /^\d{1,3}(\.\d{1,3}){3}$/.test(host)
    || host.startsWith("[")
    || !host.includes(".");
}

function storeHostOk(host) {
  if (!host || host.length > 253) return false;
  const labels = host.split(".");
  if (!labels.every(label => /^[a-z0-9-]{1,63}$/i.test(label) && !label.startsWith("-") && !label.endsWith("-"))) return false;
  return !privateStoreHost(host.toLowerCase());
}

function storePathOk(path) {
  return path.split("/").every(part => /^[A-Za-z0-9._-]*$/.test(part) && part !== "." && part !== "..");
}

function cleanFileStore(input) {
  const text = String(input || "").trim();
  if (text.length > FILE_STORE_MAX_CHARS || !text.startsWith("https://")) return null;
  const rest = text.slice("https://".length).replace(/\/+$/, "");
  const slash = rest.indexOf("/");
  const host = slash >= 0 ? rest.slice(0, slash) : rest;
  const path = slash >= 0 ? rest.slice(slash + 1) : null;
  if (!storeHostOk(host)) return null;
  if (path !== null && (!path || !storePathOk(path))) return null;
  return path === null ? `https://${host.toLowerCase()}` : `https://${host.toLowerCase()}/${path}`;
}

function fileStoreLabel(url) {
  return url ? url.replace(/^https:\/\//, "") : "";
}

function fileStoreStatus(server) {
  if (!server || !server.file_store) return "Files here travel between members' devices only.";
  return `Members' apps also keep a locked copy of each file up to 30 MB at ${fileStoreLabel(server.file_store)}.`;
}

if (typeof module !== "undefined") {
  module.exports = { FILE_STORE_MAX_CHARS, FILE_STORE_GUIDE, FILE_STORE_INVALID, cleanFileStore, fileStoreLabel, fileStoreStatus };
}
