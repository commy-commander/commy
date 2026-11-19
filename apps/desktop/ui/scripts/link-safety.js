const ALLOWED_LINKS_KEY = "commy.trustedLinkDomains";
const MAX_ALLOWED_LINK_DOMAINS = 500;

const LINK_SIGN_TEXT = {
  get known_bad() { return t("ui.it-is-on-a-list-of-sites-known-for"); },
  get disguised() { return t("ui.the-link-hides-where-it-really-goes"); },
  get text_mismatch() { return t("ui.the-link-s-text-names-a-different-site"); },
  get lookalike() { return t("ui.the-address-imitates-a-well-known-site"); },
  get brand_name() { return t("ui.the-address-borrows-the-name-of-a-well"); },
  get international() { return t("ui.the-address-uses-letters-from-other"); },
  get address() { return t("ui.it-opens-a-numeric-address-instead-of-a"); },
};

const LEAVING_TONES = {
  ask: {
    tone: "plain",
    get title() { return t("ui.leaving-commy"); },
    get intro() { return t("ui.this-link-opens-a-site-outside-commy"); },
    get openLabel() { return t("ui.open-site"); },
  },
  warn: {
    tone: "warn",
    get title() { return t("ui.careful-with-this-link"); },
    get intro() { return t("ui.this-link-may-not-go-where-it-seems-to"); },
    get openLabel() { return t("ui.open-anyway"); },
  },
  block: {
    tone: "block",
    get title() { return t("ui.harmful-site-blocked"); },
    get intro() { return t("ui.commy-checked-this-link-on-this-device"); },
    get openLabel() { return t("ui.open-anyway"); },
  },
};

function cleanDomain(value) {
  return typeof value === "string" ? value.trim().toLowerCase().replace(/\.+$/, "") : "";
}

function readAllowedDomains(raw) {
  let parsed;
  try {
    parsed = JSON.parse(raw || "[]");
  } catch {
    return [];
  }
  if (!Array.isArray(parsed)) return [];
  const domains = [...new Set(parsed.map(cleanDomain).filter(Boolean))];
  return domains.slice(-MAX_ALLOWED_LINK_DOMAINS);
}

function withAllowedDomain(domains, domain) {
  const key = cleanDomain(domain);
  const rest = (domains || []).filter(d => d !== key);
  if (!key) return rest;
  return [...rest, key].slice(-MAX_ALLOWED_LINK_DOMAINS);
}

function writeAllowedDomains(domains) {
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

const LINK_SCHEMES = ["http:", "https:"];
const LINK_REFUSED = "only http and https links can be opened";

function openLinkInNewTab(win, url) {
  let parsed;
  try {
    parsed = new URL(url);
  } catch {
    throw LINK_REFUSED;
  }
  if (!LINK_SCHEMES.includes(parsed.protocol)) throw LINK_REFUSED;
  win.open(parsed.href, "_blank", "noopener,noreferrer");
  return null;
}

function allowedLinksSummary(domains) {
  const count = (domains || []).length;
  if (count === 0) return t("ui.every-site-outside-commy-asks-before-it");
  return t("ui.without-asking-on-this-device", { count, value: count === 1 ? "site opens" : "sites open" });
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { ALLOWED_LINKS_KEY, MAX_ALLOWED_LINK_DOMAINS, LINK_SIGN_TEXT, readAllowedDomains, withAllowedDomain, writeAllowedDomains, shownHostParts, leavingPrompt, allowedLinksSummary, openLinkInNewTab, LINK_REFUSED };
}
