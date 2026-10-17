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

if (typeof module !== "undefined" && module.exports) {
  module.exports = { indexPeople, mutualServers };
}
