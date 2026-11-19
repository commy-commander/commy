const VOICE_INVITE_SLOTS = 6;

function voiceInviteSummary(users, { slots = VOICE_INVITE_SLOTS } = {}) {
  const list = Array.isArray(users) ? users.filter(Boolean) : [];
  const shown = list.slice(0, slots);
  const overflow = Math.max(0, list.length - slots);
  const names = list.map(u => (typeof u === "string" ? u : u.name) || t("ui.someone"));
  let text;
  if (!list.length) text = t("ui.nobody-is-in-the-call-yet");
  else if (list.length === 1) text = t("ui.is-in-the-call", { value: names[0] });
  else if (list.length === 2) text = t("ui.and-are-in-the-call", { value: names[0], value2: names[1] });
  else if (list.length === 3) text = t("ui.and-are-in-the-call-2", { value: names[0], value2: names[1], value3: names[2] });
  else text = t("ui.people-are-in-the-call", { count: list.length });
  return { shown, overflow, overflowLabel: overflow ? `+${overflow}` : "", count: list.length, text };
}

function voiceInviteTitle({ channelName, serverName, live }) {
  const channel = (live && live.channel_name) || channelName || "a voice channel";
  const server = (live && live.server_name) || serverName || "";
  return server ? `${channel} · ${server}` : channel;
}

function voiceInviteStatus({ known, gone, canView, users, limit }) {
  if (gone) return t("ui.this-voice-channel-is-gone");
  if (!known) return t("ui.join-to-see-who-is-here");
  if (canView === false) return t("ui.you-cannot-see-this-channel");
  const summary = voiceInviteSummary(users);
  if (limit && summary.count >= limit) return t("ui.full-2", { text: summary.text });
  return summary.text;
}

function voiceInviteAction({ known, gone, canView, inThisCall, hasLink, full }) {
  if (inThisCall) return { label: t("ui.joined"), kind: "none", enabled: false, note: t("ui.you-are-in-this-call") };
  if (gone) return { label: t("ui.join"), kind: "none", enabled: false, note: t("ui.this-voice-channel-is-gone") };
  if (known && canView === false) return { label: t("ui.join"), kind: "none", enabled: false, note: t("ui.you-cannot-see-this-channel") };
  if (known && full) return { label: t("ui.join"), kind: "none", enabled: false, note: t("ui.this-call-is-full") };
  if (known) return { label: t("ui.join"), kind: "join", enabled: true, note: t("ui.join-the-call") };
  if (hasLink) return { label: t("ui.join"), kind: "link", enabled: true, note: t("ui.join-the-server-then-the-call") };
  return { label: t("ui.join"), kind: "none", enabled: false, note: t("ui.ask-for-an-invite-to-this-server") };
}

function voiceInviteIsFull({ users, limit, inThisCall }) {
  return Boolean(limit) && !inThisCall && (users || []).length >= limit;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { VOICE_INVITE_SLOTS, voiceInviteSummary, voiceInviteTitle, voiceInviteStatus, voiceInviteAction, voiceInviteIsFull };
}
