const TRUSTED_LINKS_KEY = "commy.trustedLinkDomains";
const MAX_TRUSTED_LINK_DOMAINS = 500;

const LINK_SIGN_TEXT = {
  known_bad: "It is on a list of sites known for phishing or harmful downloads.",
  disguised: "The link hides where it really goes behind another name.",
  text_mismatch: "The link's text names a different site than the one it opens.",
  lookalike: "The address imitates a well-known site.",
  brand_name: "The address borrows the name of a well-known site.",
  international: "The address uses letters from other alphabets, which can make it look like a different site.",
  address: "It opens a numeric address instead of a site name.",
};

const LEAVING_TONES = {
  ask: {
    tone: "plain",
    title: "Leaving Commy",
    intro: "This link opens a site outside Commy. Continue only if you trust it.",
    openLabel: "Open Site",
  },
  warn: {
    tone: "warn",
    title: "Careful With This Link",
    intro: "This link may not go where it seems to. Commy checked it on this device.",
    openLabel: "Open Anyway",
  },
  block: {
    tone: "block",
    title: "Harmful Site Blocked",
    intro: "Commy checked this link on this device and stopped it.",
    openLabel: "Open Anyway",
  },
};

function cleanDomain(value) {
  return typeof value === "string" ? value.trim().toLowerCase().replace(/\.+$/, "") : "";
}

function readTrustedDomains(raw) {
  let parsed;
  try {
    parsed = JSON.parse(raw || "[]");
  } catch {
    return [];
  }
  if (!Array.isArray(parsed)) return [];
  const domains = [...new Set(parsed.map(cleanDomain).filter(Boolean))];
  return domains.slice(-MAX_TRUSTED_LINK_DOMAINS);
}

function withTrustedDomain(domains, domain) {
  const key = cleanDomain(domain);
  const rest = (domains || []).filter(d => d !== key);
  if (!key) return rest;
  return [...rest, key].slice(-MAX_TRUSTED_LINK_DOMAINS);
}

function writeTrustedDomains(domains) {
  return JSON.stringify(domains || []);
}

function shownHostParts(check) {
  const shown = String((check && check.shown_host) || "");
  const siteLabels = String((check && check.site) || "").split(".").filter(Boolean).length;
  const labels = shown.split(".");
  if (!siteLabels || siteLabels >= labels.length || check.signs.includes("address")) return { prefix: "", site: shown };
  const site = labels.slice(-siteLabels).join(".");
  return { prefix: shown.slice(0, shown.length - site.length), site };
}

function leavingPrompt(check) {
  const level = LEAVING_TONES[check.level] ? check.level : "ask";
  const signs = Array.isArray(check.signs) ? check.signs : [];
  return {
    ...LEAVING_TONES[level],
    host: shownHostParts({ ...check, signs }),
    spelled: signs.includes("international") && check.host !== check.shown_host ? check.host : "",
    notes: signs.map(sign => LINK_SIGN_TEXT[sign]).filter(Boolean),
    canTrust: Boolean(check.can_trust),
  };
}

function trustedLinksSummary(domains) {
  const count = (domains || []).length;
  if (count === 0) return "Every site outside Commy asks before it opens.";
  return `${count} ${count === 1 ? "site opens" : "sites open"} without asking on this device.`;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { TRUSTED_LINKS_KEY, MAX_TRUSTED_LINK_DOMAINS, LINK_SIGN_TEXT, readTrustedDomains, withTrustedDomain, writeTrustedDomains, shownHostParts, leavingPrompt, trustedLinksSummary };
}
