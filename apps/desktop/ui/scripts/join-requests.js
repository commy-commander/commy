const JOIN_NOTE_MAX = 300;

function joinNoteLeft(text) {
  return JOIN_NOTE_MAX - [...String(text || "").trim()].length;
}

function joinRequestsLabel(count) {
  return count ? t("ui.join-requests-2", { count }) : t("ui.join-requests");
}

function joinWaitStatus(wait) {
  if (!wait) return { title: "", text: "" };
  if (wait.turned_away) {
    return { title: t("ui.did-not-let-you-in", { name: wait.name }), text: t("ui.a-moderator-turned-down-your-request-you") };
  }
  return {
    title: t("ui.waiting-to-join", { name: wait.name }),
    text: t("ui.a-moderator-will-let-you-in-you-can"),
  };
}

function joinWaitMembers(count) {
  const n = Number(count) || 0;
  return t(n === 1 ? "ui.n.members.one" : "ui.n.members.many", { count: n });
}

function newJoinRequests(before, after) {
  const known = new Set((before || []).map(r => r.id));
  return (after || []).filter(r => !known.has(r.id));
}

function joinRequestToast(server, fresh) {
  if (!fresh.length) return "";
  if (fresh.length === 1) return t("ui.asked-to-join", { value: fresh[0].name, server });
  return t("ui.people-asked-to-join", { count: fresh.length, server });
}

if (typeof module !== "undefined") {
  module.exports = { JOIN_NOTE_MAX, joinNoteLeft, joinRequestsLabel, joinWaitStatus, joinWaitMembers, newJoinRequests, joinRequestToast };
}
