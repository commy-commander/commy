const PHONE_TABS = ["home", "messages", "live", "you"];
const HORIZON = { baseline: 30, swell: 9, spread: 25, pingSpread: 700, step: 3, overhang: 6 };
const HORIZON_SPRINGS = { slide: { k: 230, c: 21 }, hop: { k: 420, c: 23 }, knock: { k: 260, c: 17 } };
const HANG_UP_FUSE_MS = 3000;
const STACK_MAX = 4;

function horizonY(x, centre, swell, pingCentre, ping) {
  const main = swell * Math.exp(-((x - centre) ** 2) / (2 * HORIZON.spread ** 2));
  const knock = ping && pingCentre !== null ? ping * Math.exp(-((x - pingCentre) ** 2) / HORIZON.pingSpread) : 0;
  return HORIZON.baseline - main - knock;
}

function horizonPath(width, centre, swell = HORIZON.swell, pingCentre = null, ping = 0) {
  const parts = [];
  for (let x = -HORIZON.overhang; x <= width + HORIZON.overhang; x += HORIZON.step) {
    parts.push(`${parts.length ? "L" : "M"}${x} ${horizonY(x, centre, swell, pingCentre, ping).toFixed(2)}`);
  }
  return parts.join("");
}

function horizonLift(tabCentre, centre, swell = HORIZON.swell) {
  return -swell * Math.exp(-((tabCentre - centre) ** 2) / (2 * HORIZON.spread ** 2));
}

function springStep(spring, seconds) {
  const dt = Math.min(0.034, Math.max(0.001, seconds));
  const slices = Math.ceil(dt / 0.004);
  const h = dt / slices;
  let { x, v } = spring;
  for (let i = 0; i < slices; i++) {
    v += (-spring.k * (x - spring.target) - spring.c * v) * h;
    x += v * h;
  }
  const settled = Math.abs(x - spring.target) < 0.03 && Math.abs(v) < 0.6;
  return settled ? { ...spring, x: spring.target, v: 0, settled } : { ...spring, x, v, settled };
}

function tabForConversation(view) {
  return view === "dm" ? "messages" : "home";
}

function phoneTabAfter(tab, { pane, view }) {
  const known = PHONE_TABS.includes(tab) ? tab : "home";
  if (pane !== "list") return tabForConversation(view);
  if (known === "live" || known === "you") return known;
  return tabForConversation(view);
}

function hangUpTap(armed) {
  return armed ? "leave" : "arm";
}

function badgeText(count) {
  const n = Math.floor(Number(count) || 0);
  if (n <= 0) return "";
  return n > 99 ? "99+" : String(n);
}

function roomLine(count, connected) {
  if (connected) return "You're connected";
  if (count <= 0) return "";
  return count === 1 ? "1 person in the room" : `${count} in the room`;
}

function liveSummary(streams, rooms) {
  if (streams > 0) return streams === 1 ? "1 stream, right now" : `${streams} streams, right now`;
  if (rooms > 0) return rooms === 1 ? "1 room has people in it" : `${rooms} rooms have people in them`;
  return "";
}

function roomPeople(channel) {
  const names = channel.voice_users || [];
  const ids = channel.voice_user_ids || [];
  return names.map((name, i) => ({ id: ids[i] || null, name }));
}

function liveRooms(servers, me = {}) {
  const rooms = [];
  for (const server of servers || []) {
    if (!server || server.group) continue;
    const shares = server.screen_shares || [];
    for (const channel of server.channels || []) {
      if (channel.kind !== "voice" && channel.kind !== "stage") continue;
      const people = roomPeople(channel);
      if (!people.length) continue;
      const streams = shares
        .filter(share => (share.channel ? share.channel === channel.id : people.some(person => person.id === share.user)))
        .map(share => ({ userId: share.user, name: share.name, mine: share.user === me.userId }));
      rooms.push({
        serverId: server.id,
        serverName: server.name,
        channelId: channel.id,
        channelName: channel.name,
        people,
        streams,
        here: Boolean(me.channelId && me.channelId === channel.id),
      });
    }
  }
  return rooms;
}

function liveStreams(rooms) {
  return rooms.flatMap(room => room.streams.map(stream => ({ ...stream, room })));
}

function stackPeople(people, max = STACK_MAX) {
  const shown = people.slice(0, max);
  return { shown, more: Math.max(0, people.length - shown.length) };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    PHONE_TABS,
    HORIZON,
    HORIZON_SPRINGS,
    HANG_UP_FUSE_MS,
    STACK_MAX,
    horizonY,
    horizonPath,
    horizonLift,
    springStep,
    tabForConversation,
    phoneTabAfter,
    hangUpTap,
    badgeText,
    roomLine,
    liveSummary,
    roomPeople,
    liveRooms,
    liveStreams,
    stackPeople,
  };
}
