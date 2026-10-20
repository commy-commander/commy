const SENSITIVE_KEY = "commy.sensitiveConfirmed";
const SENSITIVE_PREVIEW = "Age-restricted channel";

function isSensitiveChannel(channel) {
  return Boolean(channel && channel.sensitive && (channel.kind === "text" || channel.kind === "announcement"));
}

function readConfirmed(raw) {
  try {
    const parsed = JSON.parse(raw || "[]");
    return new Set(Array.isArray(parsed) ? parsed.filter(id => typeof id === "string") : []);
  } catch {
    return new Set();
  }
}

function writeConfirmed(confirmed) {
  return JSON.stringify([...(confirmed || [])]);
}

function sensitiveGated({ view, channel, confirmed }) {
  if (view !== "server" || !isSensitiveChannel(channel)) return false;
  return !(confirmed && confirmed.has(channel.id));
}

function showsSensitiveBadge(channel) {
  return isSensitiveChannel(channel);
}

function sensitivePreview({ channel, confirmed, text }) {
  return sensitiveGated({ view: "server", channel, confirmed }) ? SENSITIVE_PREVIEW : text;
}

function sensitiveNotice(info, channelId, confirmed) {
  if (!info || !info.sensitive || (confirmed && confirmed.has(channelId))) return info;
  return { ...info, content: SENSITIVE_PREVIEW, has_attachments: false };
}

function sensitiveGateText(channelName) {
  return {
    title: "This channel is age-restricted",
    body: `#${channelName} is for people 18 or older. Continue only if that is you; you will not be asked again on this device.`,
  };
}

function fallbackChannel({ channels, previous, confirmed }) {
  const open = c => c && (c.kind === "text" || c.kind === "announcement") && c.can_view !== false && !sensitiveGated({ view: "server", channel: c, confirmed });
  const list = channels || [];
  if (previous && open(previous) && list.some(c => c.id === previous.id)) return list.find(c => c.id === previous.id);
  return list.find(open) || null;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    SENSITIVE_KEY, SENSITIVE_PREVIEW, isSensitiveChannel, readConfirmed, writeConfirmed, sensitiveGated,
    showsSensitiveBadge, sensitivePreview, sensitiveNotice, sensitiveGateText, fallbackChannel,
  };
}
