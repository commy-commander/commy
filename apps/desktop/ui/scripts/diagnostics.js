"use strict";

const CONNECTION_BADGES = {
  good: { label: "Good", tone: "good" },
  fair: { label: "Fair", tone: "ok" },
  poor: { label: "Poor", tone: "bad" },
  offline: { label: "Offline", tone: "bad" },
};

const CONNECTION_SUMMARIES = {
  good: "Calls and streams should run smoothly.",
  fair: "Calls work, though they may lag a little or look less sharp.",
  poor: "Calls may lag or cut out on this network.",
  offline: "Commy can't reach its network. Check your internet connection and try again.",
};

const DIRECT_ANSWERS = {
  open: "Yes",
  limited: "Sometimes",
  blocked: "No, using the relay",
  unchecked: "Not checked",
};

const DIRECT_DETAILS = {
  open: "Your network lets Commy connect straight to other people, so calls and streams usually skip the relay.",
  limited: "Your network makes direct connections hard. Some calls and streams will go through Commy's relay instead.",
  blocked: "Your network blocks direct connections, so calls and streams go through Commy's relay. They still work, and streams are sent at a lower frame rate.",
  unchecked: "Commy couldn't check direct connections on this network yet. Calls still connect, directly when they can and through Commy's relay when they can't.",
};

const BROWSER_DIRECT = "Browsers always use the relay";
const BROWSER_DETAIL = "Browsers can't connect straight to other people, so the web app always goes through Commy's relay. The desktop app can often connect directly.";

function relayRoundTripText(check) {
  if (Number.isFinite(check.relay_rtt_ms)) return `${Math.round(check.relay_rtt_ms)} ms`;
  return check.relay_reachable ? "Not measured" : "Not reachable";
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
