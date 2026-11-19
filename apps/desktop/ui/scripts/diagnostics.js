"use strict";

const CONNECTION_BADGES = {
  good: { get label() { return t("ui.signal.good"); }, tone: "good" },
  fair: { get label() { return t("ui.signal.fair"); }, tone: "ok" },
  poor: { get label() { return t("ui.signal.poor"); }, tone: "bad" },
  offline: { get label() { return t("ui.offline-2"); }, tone: "bad" },
};

const CONNECTION_SUMMARIES = {
  get good() { return t("ui.calls-and-streams-should-run-smoothly"); },
  get fair() { return t("ui.calls-work-though-they-may-lag-a-little"); },
  get poor() { return t("ui.calls-may-lag-or-cut-out-on-this-network"); },
  get offline() { return t("ui.commy-can-t-reach-its-network-check-your"); },
};

const DIRECT_ANSWERS = {
  get open() { return t("ui.yes"); },
  get limited() { return t("ui.sometimes"); },
  get blocked() { return t("ui.no-using-the-relay"); },
  get unchecked() { return t("ui.not-checked"); },
};

const DIRECT_DETAILS = {
  get open() { return t("ui.your-network-lets-commy-connect-straight"); },
  get limited() { return t("ui.your-network-makes-direct-connections"); },
  get blocked() { return t("ui.your-network-blocks-direct-connections"); },
  get unchecked() { return t("ui.commy-couldn-t-check-direct-connections"); },
};

const BROWSER_DIRECT = "Browsers always use the relay";
const BROWSER_DETAIL = "Browsers can't connect straight to other people, so the web app always goes through Commy's relay. The desktop app can often connect directly.";

function relayRoundTripText(check) {
  if (Number.isFinite(check.relay_rtt_ms)) return `${Math.round(check.relay_rtt_ms)} ms`;
  return check.relay_reachable ? t("ui.not-measured") : t("ui.not-reachable");
}

function describeConnectionCheck(check, { browser = false } = {}) {
  const quality = CONNECTION_BADGES[check && check.quality] ? check.quality : "offline";
  const direct = DIRECT_ANSWERS[check && check.direct] ? check.direct : "unchecked";
  const badge = CONNECTION_BADGES[quality];
  const inBrowser = browser && direct === "unchecked";
  const offline = quality === "offline";
  return {
    badge: badge.label,
    tone: badge.tone,
    summary: CONNECTION_SUMMARIES[quality],
    direct: offline ? "-" : inBrowser ? BROWSER_DIRECT : DIRECT_ANSWERS[direct],
    relay: relayRoundTripText(check || {}),
    detail: offline ? "" : inBrowser ? BROWSER_DETAIL : DIRECT_DETAILS[direct],
  };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { describeConnectionCheck, relayRoundTripText };
}
