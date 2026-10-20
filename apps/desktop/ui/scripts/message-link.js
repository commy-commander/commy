const UUID = "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}";
const MESSAGE_LINK_RE = new RegExp(`commy:\\/\\/m\\/(${UUID})\\/(${UUID})\\/(${UUID})\\b`, "i");

function messageLink({ server, channel, message }) {
  return `commy://m/${server}/${channel}/${message}`;
}

function parseMessageLink(text) {
  const m = MESSAGE_LINK_RE.exec(String(text || ""));
  if (!m) return null;
  return { server: m[1].toLowerCase(), channel: m[2].toLowerCase(), message: m[3].toLowerCase(), link: m[0] };
}

function isBareMessageLink(text) {
  const found = parseMessageLink(text);
  return Boolean(found) && found.link === String(text).trim();
}

function forwardBody({ content, author, where }) {
  const lines = String(content || "").split("\n");
  const from = where ? `${author} in ${where}` : author;
  return [`**Forwarded** from ${from}`, ...lines.map(l => `> ${l}`)].join("\n");
}

function forwardTargets({ servers, dms, query }) {
  const q = String(query || "").trim().toLowerCase();
  const hit = (...parts) => !q || parts.some(p => String(p || "").toLowerCase().includes(q));
  const out = [];
  for (const server of servers || []) {
    for (const channel of server.channels || []) {
      if (channel.kind !== "text" && channel.kind !== "announcement") continue;
      if (channel.can_send === false) continue;
      if (server.group) {
        if (hit(server.name)) out.push({ kind: "group", server: server.id, channel: channel.id, name: server.name, sub: "Group" });
        break;
      }
      if (hit(channel.name, server.name)) out.push({ kind: "channel", server: server.id, channel: channel.id, name: `#${channel.name}`, sub: server.name });
    }
  }
  for (const dm of dms || []) {
    if (dm.kind !== "inbox") continue;
    if (hit(dm.name)) out.push({ kind: "dm", user: dm.user, name: dm.name, sub: "Direct message" });
  }
  return out;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { MESSAGE_LINK_RE, messageLink, parseMessageLink, isBareMessageLink, forwardBody, forwardTargets };
}
