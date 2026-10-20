const STAGE_KIND = "stage";

function isVoiceKind(kind) {
  return kind === "voice" || kind === STAGE_KIND;
}

function isStageChannel(channel) {
  return Boolean(channel) && channel.kind === STAGE_KIND;
}

function isStageSpeaker(channel, userId) {
  return isStageChannel(channel) && Boolean(userId) && (channel.speakers || []).includes(userId);
}

function stageRole(channel, userId, canManage = false) {
  if (!isStageChannel(channel)) return null;
  return canManage || isStageSpeaker(channel, userId) ? "speaker" : "audience";
}

function stageRoster(channel) {
  const ids = channel.voice_user_ids || [];
  const hands = channel.hands || [];
  return (channel.voice_users || []).map((name, index) => ({
    index,
    id: ids[index] || null,
    name,
    since: (channel.voice_since || [])[index] || null,
    muted: Boolean((channel.voice_muted || [])[index]),
    deafened: Boolean((channel.voice_deafened || [])[index]),
    hand: Boolean(ids[index]) && hands.includes(ids[index]),
  }));
}

function orderHandsFirst(entries, hands) {
  const rank = id => {
    const at = hands.indexOf(id);
    return at < 0 ? hands.length : at;
  };
  return [...entries].sort((a, b) => Number(b.hand) - Number(a.hand) || rank(a.id) - rank(b.id) || a.index - b.index);
}

function stageSections(channel) {
  const roster = stageRoster(channel);
  if (!isStageChannel(channel)) return { speakers: roster, audience: [] };
  const speakers = roster.filter(e => isStageSpeaker(channel, e.id));
  const audience = orderHandsFirst(roster.filter(e => !isStageSpeaker(channel, e.id)), channel.hands || []);
  return { speakers, audience };
}

function handState(channel, userId, canManage = false) {
  const role = stageRole(channel, userId, canManage);
  if (!role) return null;
  const raised = Boolean(userId) && (channel.hands || []).includes(userId);
  return {
    role,
    raised,
    text: role === "speaker" ? "On the stage" : "In the audience",
    detail: role === "speaker" ? "You are a speaker here" : "You are in the audience, only speakers are heard",
    label: raised ? "Lower hand" : "Raise hand",
    title: role === "speaker" ? "" : raised ? "Let the hosts know you no longer want to speak" : "Ask the hosts to let you speak",
  };
}

function stageChannelOf(server, userId) {
  if (!server || !userId) return null;
  return (server.channels || []).find(c => isStageChannel(c) && (c.voice_user_ids || []).includes(userId)) || null;
}

function stageMenuAction(channel, memberId, canManage = false) {
  if (!canManage || !isStageChannel(channel) || !(channel.voice_user_ids || []).includes(memberId)) return null;
  const speaker = isStageSpeaker(channel, memberId);
  return speaker
    ? { speaker: false, label: "Move to audience", done: "moved to the audience" }
    : { speaker: true, label: "Invite to speak", done: "invited to speak" };
}

function stageCreateHint(kind) {
  if (kind !== STAGE_KIND) return null;
  return { placeholder: "Town Hall", icon: "stage" };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    STAGE_KIND, isVoiceKind, isStageChannel, isStageSpeaker, stageRole, stageRoster, stageSections, handState,
    stageChannelOf, stageMenuAction, stageCreateHint,
  };
}
