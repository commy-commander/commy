const VOICE_INVITE_SLOTS = 6;

function voiceInviteSummary(users, { slots = VOICE_INVITE_SLOTS } = {}) {
  const list = Array.isArray(users) ? users.filter(Boolean) : [];
  const shown = list.slice(0, slots);
  const overflow = Math.max(0, list.length - slots);
  const names = list.map(u => (typeof u === "string" ? u : u.name) || "Someone");
  let text;
  if (!list.length) text = "Nobody is in the call yet";
  else if (list.length === 1) text = `${names[0]} is in the call`;
  else if (list.length === 2) text = `${names[0]} and ${names[1]} are in the call`;
  else if (list.length === 3) text = `${names[0]}, ${names[1]} and ${names[2]} are in the call`;
  else text = `${list.length} people are in the call`;
  return { shown, overflow, overflowLabel: overflow ? `+${overflow}` : "", count: list.length, text };
}

function voiceInviteTitle({ channelName, serverName, live }) {
  const channel = (live && live.channel_name) || channelName || "a voice channel";
  const server = (live && live.server_name) || serverName || "";
  return server ? `${channel} · ${server}` : channel;
}

function voiceInviteStatus({ known, gone, canView, users, limit }) {
  if (gone) return "This voice channel is gone";
  if (!known) return "Join to see who is here";
  if (canView === false) return "You cannot see this channel";
  const summary = voiceInviteSummary(users);
  if (limit && summary.count >= limit) return `${summary.text} · full`;
  return summary.text;
}

function voiceInviteAction({ known, gone, canView, inThisCall, hasLink, full }) {
  if (inThisCall) return { label: "Joined", kind: "none", enabled: false, note: "You are in this call" };
  if (gone) return { label: "Join", kind: "none", enabled: false, note: "This voice channel is gone" };
  if (known && canView === false) return { label: "Join", kind: "none", enabled: false, note: "You cannot see this channel" };
  if (known && full) return { label: "Join", kind: "none", enabled: false, note: "This call is full" };
  if (known) return { label: "Join", kind: "join", enabled: true, note: "Join the call" };
  if (hasLink) return { label: "Join", kind: "link", enabled: true, note: "Join the server, then the call" };
  return { label: "Join", kind: "none", enabled: false, note: "Ask for an invite to this server" };
}

function voiceInviteIsFull({ users, limit, inThisCall }) {
  return Boolean(limit) && !inThisCall && (users || []).length >= limit;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { VOICE_INVITE_SLOTS, voiceInviteSummary, voiceInviteTitle, voiceInviteStatus, voiceInviteAction, voiceInviteIsFull };
}
