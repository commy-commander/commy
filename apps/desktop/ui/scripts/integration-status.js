const INTEGRATION_FRESH_MS = 10 * 60e3;
const INTEGRATION_KINDS = ["gif-search", "game-database", "file-storage"];

const INTEGRATION_WORDS = {
  busy: "Checking the connection",
  working: "Connected and working",
  broken: "Connected, but not working",
  "gif-search": {
    builtin: "Working with the built-in search. No key is needed.",
    off: "Not connected. Add a key to turn GIF search on here.",
    down: "The built-in search is not working",
  },
  "game-database": {
    builtin: "Working through your Commy account. No login is needed.",
    off: "Not connected. Add a login to turn game search on here.",
    down: "Game search through your Commy account is not working",
  },
  "file-storage": {
    off: "Not connected",
    unpaired: "Connected, but the worker has not been set up for this server. Enter the setup code and save.",
  },
};

function integrationReason(lead, reason) {
  const text = String(reason || "").trim().replace(/[.\s]+$/, "");
  return text ? `${lead}: ${text}` : lead;
}

function integrationStatus(kind, probe) {
  const words = INTEGRATION_WORDS[kind];
  if (!words) return { tone: "off", text: "" };
  if (!probe) return { tone: "busy", text: INTEGRATION_WORDS.busy };
  if (probe.configured) {
    if (probe.ok) return { tone: "ok", text: INTEGRATION_WORDS.working };
    if (probe.reason === "unpaired" && words.unpaired) return { tone: "warn", text: words.unpaired };
    return { tone: "warn", text: integrationReason(INTEGRATION_WORDS.broken, probe.reason === "empty" ? "" : probe.reason) };
  }
  if (probe.ok && words.builtin) return { tone: "ok", text: words.builtin };
  if (!probe.reason || probe.reason === "no-key" || probe.reason === "empty" || !words.down) return { tone: "off", text: words.off };
  return { tone: "warn", text: integrationReason(words.down, probe.reason) };
}

function createIntegrationChecks(now = () => Date.now()) {
  const seen = new Map();
  return {
    fresh(key) {
      const hit = seen.get(key);
      return hit && now() - hit.at < INTEGRATION_FRESH_MS ? hit.status : null;
    },
    keep(key, status) {
      seen.set(key, { at: now(), status });
      return status;
    },
    clear() {
      seen.clear();
    },
  };
}

if (typeof module !== "undefined") {
  module.exports = { INTEGRATION_FRESH_MS, INTEGRATION_KINDS, INTEGRATION_WORDS, integrationStatus, createIntegrationChecks };
}
