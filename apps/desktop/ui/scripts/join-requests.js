const JOIN_NOTE_MAX = 300;

function joinNoteLeft(text) {
  return JOIN_NOTE_MAX - [...String(text || "").trim()].length;
}

function joinRequestsLabel(count) {
  return count ? `Join Requests (${count})` : "Join Requests";
}

function joinWaitStatus(wait) {
  if (!wait) return { title: "", text: "" };
  if (wait.turned_away) {
    return { title: `${wait.name} did not let you in`, text: "A moderator turned down your request. You can remove it from your list." };
  }
  return {
    title: `Waiting to join ${wait.name}`,
    text: "A moderator will let you in. You can close this: Commy keeps checking and brings you in as soon as they say yes.",
  };
}

function joinWaitMembers(count) {
  const n = Number(count) || 0;
  return n === 1 ? "1 member" : `${n} members`;
}

function newJoinRequests(before, after) {
  const known = new Set((before || []).map(r => r.id));
  return (after || []).filter(r => !known.has(r.id));
}

function joinRequestToast(server, fresh) {
  if (!fresh.length) return "";
  if (fresh.length === 1) return `${fresh[0].name} asked to join ${server}`;
  return `${fresh.length} people asked to join ${server}`;
}

if (typeof module !== "undefined") {
  module.exports = { JOIN_NOTE_MAX, joinNoteLeft, joinRequestsLabel, joinWaitStatus, joinWaitMembers, newJoinRequests, joinRequestToast };
}
