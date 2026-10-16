// The people index: everyone you share a server with, folded across
// servers, with the servers you have in common. Built on the device from
// its own server views; the account server never learns who is where.
// Kept free of the DOM so it can be tested with node.

// Folds server views (each with id, name and members) into a Map of
// person id -> { id, name, presence, ..., servers: [{ id, name }] }.
// `meId` is left out, since you are not in common with yourself.
function indexPeople(servers, meId) {
  const people = new Map();
  for (const server of servers) {
    for (const member of server.members || []) {
      if (meId && member.id === meId) continue;
      const person = people.get(member.id) || {
        id: member.id, name: member.name, presence: member.presence, presence_until: member.presence_until ?? null,
        custom_status: member.custom_status, status_emoji: member.status_emoji, activity: member.activity, servers: [],
      };
      // Prefer the liveliest presence seen across servers.
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

// Servers you share with someone, as [{ id, name }].
function mutualServers(people, userId) {
  const person = people.get(userId);
  return person ? person.servers.slice() : [];
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { indexPeople, mutualServers };
}
