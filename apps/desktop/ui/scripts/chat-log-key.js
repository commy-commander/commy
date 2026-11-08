const VOICE_ONLY_CHANNEL_FIELDS = ["voice_users", "voice_user_ids", "voice_since", "voice_muted", "voice_deafened", "voice_status", "speakers", "hands", "activity"];
const VOICE_ONLY_MEMBER_FIELDS = ["voice_channel", "voice_since", "voice_muted", "voice_deafened", "presence", "presence_until"];
const VOICE_ONLY_SERVER_FIELDS = ["screen_shares", "cameras", "server_muted", "server_deafened"];

function withoutFields(record, fields) {
  const kept = { ...record };
  for (const field of fields) delete kept[field];
  return kept;
}

function serverAsChatSeesIt(server) {
  if (!server) return null;
  const kept = withoutFields(server, VOICE_ONLY_SERVER_FIELDS);
  kept.channels = (server.channels || []).map(c => withoutFields(c, VOICE_ONLY_CHANNEL_FIELDS));
  kept.members = (server.members || []).map(m => withoutFields(m, VOICE_ONLY_MEMBER_FIELDS));
  return kept;
}

function chatLogKey(channelId, readLine, messages, server) {
  return JSON.stringify([channelId, readLine ?? null, messages, serverAsChatSeesIt(server)]);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { chatLogKey };
}
