function indexPeople(servers, meId) {
  const people = new Map();
  for (const server of servers) {
    for (const member of server.members || []) {
      if (meId && member.id === meId) continue;
      const person = people.get(member.id) || {
        id: member.id, name: member.name, presence: member.presence, presence_until: member.presence_until ?? null,
        custom_status: member.custom_status, status_emoji: member.status_emoji, activity: member.activity, servers: [],
      };
      if (person.presence === "offline" && member.presence !== "offline") person.presence = member.presence;
      person.presence_until ||= member.presence_until ?? null;
      person.custom_status ||= member.custom_status;
      person.status_emoji ||= member.status_emoji;
      person.activity ||= member.activity;
      if (!person.servers.some(s => s.id === server.id)) person.servers.push({ id: server.id, name: server.name });
      people.set(member.id, person);
    }
  }
  return people;
}

function mutualServers(people, userId) {
  const person = people.get(userId);
  return person ? person.servers.slice() : [];
}

function groupMembers(members, roles) {
  const hoisted = (roles || []).filter(r => r.hoist).sort((a, b) => b.position - a.position);
  const isOnline = m => m.presence !== "offline" && m.presence !== "invisible";
  const groups = hoisted.map(role => ({ id: role.id, title: role.name, color: role.color || null, members: [] }));
  const online = { id: "online", title: "Online", color: null, members: [] };
  const offline = { id: "offline", title: "Offline", color: null, members: [], dim: true };
  for (const member of members) {
    if (!isOnline(member)) {
      offline.members.push(member);
      continue;
    }
    const held = member.roles || [];
    const index = hoisted.findIndex(role => held.includes(role.id));
    (index === -1 ? online : groups[index]).members.push(member);
  }
  return [...groups, online, offline].filter(g => g.members.length);
}

function inviteCandidates(people, friendIds, memberIds, query) {
  const needle = String(query || "").trim().toLowerCase();
  const friends = new Set(friendIds);
  const inServer = new Set(memberIds);
  return [...people.values()]
    .filter(p => !inServer.has(p.id))
    .filter(p => !needle || p.name.toLowerCase().includes(needle))
    .map(p => ({ ...p, friend: friends.has(p.id) }))
    .sort((a, b) => b.friend - a.friend || (a.presence === "offline") - (b.presence === "offline") || a.name.localeCompare(b.name));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { indexPeople, mutualServers, groupMembers, inviteCandidates };
}
