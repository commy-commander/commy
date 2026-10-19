const AUTOMOD_MAX_TERMS = 100;
const AUTOMOD_MAX_TERM_CHARS = 40;
const AUTOMOD_MAX_TERMS_BYTES = 1500;
const AUTOMOD_MAX_EXEMPT_ROLES = 20;
const MENTION_LIMIT_CHOICES = [0, 3, 5, 10, 20];
const BURST_CHOICES = [
  { messages: 3, seconds: 5 },
  { messages: 5, seconds: 10 },
  { messages: 10, seconds: 30 },
  { messages: 20, seconds: 60 },
];

function parseBlockedTerms(text) {
  const seen = new Set();
  const terms = [];
  for (const line of String(text || "").split("\n")) {
    const term = line.trim().replace(/\s+/g, " ");
    const key = term.toLowerCase();
    if (!term || seen.has(key)) continue;
    seen.add(key);
    terms.push(term);
  }
  return terms;
}

function termsUsage(terms) {
  const bytes = terms.reduce((sum, t) => sum + new TextEncoder().encode(t.toLowerCase()).length, 0);
  const tooLong = terms.filter(t => t.length > AUTOMOD_MAX_TERM_CHARS);
  return {
    count: terms.length,
    bytes,
    tooLong,
    over: terms.length > AUTOMOD_MAX_TERMS || bytes > AUTOMOD_MAX_TERMS_BYTES || tooLong.length > 0,
  };
}

function termsUsageText(usage) {
  if (usage.tooLong.length) return `Too long: ${usage.tooLong[0]}`;
  return `${usage.count} / ${AUTOMOD_MAX_TERMS} words, ${usage.bytes} / ${AUTOMOD_MAX_TERMS_BYTES} characters`;
}

function burstKey(burst) {
  return burst ? `${burst.messages}/${burst.seconds}` : "off";
}

function burstLabel(burst) {
  return burst ? `More than ${burst.messages} messages in ${burst.seconds} seconds` : "Off";
}

function burstOptions(current) {
  const options = [{ value: "off", label: "Off" }, ...BURST_CHOICES.map(b => ({ value: burstKey(b), label: burstLabel(b) }))];
  if (current && !options.some(o => o.value === burstKey(current))) {
    options.push({ value: burstKey(current), label: burstLabel(current) });
  }
  return options;
}

function burstFromKey(key) {
  const [messages, seconds] = String(key || "").split("/").map(Number);
  return Number.isInteger(messages) && Number.isInteger(seconds) ? { messages, seconds } : null;
}

function mentionOptions(current) {
  const values = MENTION_LIMIT_CHOICES.includes(current || 0) ? MENTION_LIMIT_CHOICES : [...MENTION_LIMIT_CHOICES, current];
  return values.map(n => ({ value: String(n), label: n ? `More than ${n} mentions` : "Off" }));
}

function automodPayload({ termsText, blockLinks, mentionLimit, burst, exemptRoles }) {
  const mentions = Number(mentionLimit) || 0;
  return {
    blocked_terms: parseBlockedTerms(termsText),
    block_links: Boolean(blockLinks),
    max_mentions: mentions > 0 ? mentions : null,
    burst: burstFromKey(burst),
    exempt_roles: [...new Set(exemptRoles || [])],
  };
}

function automodSummary(rules) {
  if (!rules) return "Off";
  const parts = [];
  const terms = (rules.blocked_terms || []).length;
  if (terms) parts.push(terms === 1 ? "1 blocked word" : `${terms} blocked words`);
  if (rules.block_links) parts.push("links blocked");
  if (rules.max_mentions) parts.push(`mention limit ${rules.max_mentions}`);
  if (rules.burst) parts.push(`flood limit ${rules.burst.messages} in ${rules.burst.seconds}s`);
  return parts.length ? parts.join(", ") : "Off";
}

function heldLabel(reason) {
  return reason ? `Held: ${reason}` : "Held";
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    AUTOMOD_MAX_TERMS, AUTOMOD_MAX_TERM_CHARS, AUTOMOD_MAX_TERMS_BYTES, AUTOMOD_MAX_EXEMPT_ROLES,
    parseBlockedTerms, termsUsage, termsUsageText, burstKey, burstLabel, burstOptions, burstFromKey,
    mentionOptions, automodPayload, automodSummary, heldLabel,
  };
}
