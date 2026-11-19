function accessRoleNames(server, ids) {
  const roles = (server && server.roles) || [];
  return (ids || []).map(id => roles.find(r => r.id === id)).filter(Boolean).map(r => r.name);
}

function accessJoin(names) {
  if (names.length <= 1) return names[0] || "";
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

function accessWho(server, ids) {
  const names = accessRoleNames(server, ids);
  return names.length ? accessJoin(names) : "admins";
}

function channelIsPrivate(channel) {
  return Boolean(channel && channel.view_roles && channel.view_roles.length);
}

function channelAccessLock({ view, server, channel }) {
  if (view !== "server" || !channel || channel.kind === "voice" || channel.kind === "stage" || channel.kind === "category") return null;
  if (channel.can_view === false) {
    return {
      title: t("ui.you-can-t-read-this-channel"),
      text: t("ui.only-can-read-it-you-can-still-change", { value: accessWho(server, channel.view_roles) }),
    };
  }
  if (channel.can_send === false && channel.send_roles && channel.send_roles.length) {
    return {
      title: t("ui.only-can-send-messages-here", { value: accessWho(server, channel.send_roles) }),
      text: t("ui.everyone-who-can-see-this-channel-can"),
    };
  }
  if (channelIsPrivate(channel) && channel.key_ready === false) {
    return {
      title: t("ui.getting-this-channel-s-key"),
      text: t("ui.it-comes-from-a-member-who-already-has"),
    };
  }
  return null;
}

function toggleAccessRole(ids, id) {
  const list = ids || [];
  return list.includes(id) ? list.filter(x => x !== id) : [...list, id];
}

function sameAccess(a, b) {
  const left = [...(a || [])].sort();
  const right = [...(b || [])].sort();
  return left.length === right.length && left.every((id, i) => id === right[i]);
}

function accessSummary(server, ids) {
  return ids && ids.length ? accessWho(server, ids) : t("ui.everyone");
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { accessRoleNames, accessJoin, accessWho, channelIsPrivate, channelAccessLock, toggleAccessRole, sameAccess, accessSummary };
}
