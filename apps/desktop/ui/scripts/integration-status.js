const INTEGRATION_FRESH_MS = 10 * 60e3;
const INTEGRATION_KINDS = ["gif-search", "game-database", "file-storage"];

const INTEGRATION_WORDS = {
  get busy() { return t("ui.checking-the-connection"); },
  get working() { return t("ui.connected-and-working"); },
  get broken() { return t("ui.connected-but-not-working"); },
  "gif-search": {
    get builtin() { return t("ui.working-with-the-built-in-search-no-key"); },
    get off() { return t("ui.not-connected-add-a-key-to-turn-gif"); },
    get down() { return t("ui.the-built-in-search-is-not-working"); },
  },
  "game-database": {
    get builtin() { return t("ui.working-through-your-commy-account-no"); },
    get off() { return t("ui.not-connected-add-a-login-to-turn-game"); },
    get down() { return t("ui.game-search-through-your-commy-account"); },
  },
  "file-storage": {
    get off() { return t("ui.not-connected-2"); },
    get unpaired() { return t("ui.connected-but-the-worker-has-not-been"); },
  },
};

const INTEGRATION_TAB_WORDS = { get busy() { return t("ui.integration-checking"); }, get ok() { return t("ui.integration-working"); }, get warn() { return t("ui.integration-problem"); }, get off() { return t("ui.off"); } };

function integrationTabWord(tone) {
  return INTEGRATION_TAB_WORDS[tone] || "";
}

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
  module.exports = { INTEGRATION_FRESH_MS, INTEGRATION_KINDS, INTEGRATION_WORDS, INTEGRATION_TAB_WORDS, integrationTabWord, integrationStatus, createIntegrationChecks };
}
