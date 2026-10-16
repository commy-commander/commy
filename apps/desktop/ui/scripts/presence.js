// Presence text, kept free of DOM and backend so it runs in the app and
// in `node --test` alike (see ui/tests/presence.test.mjs).

const PRESENCE_LABELS = { online: "Online", idle: "Idle", do_not_disturb: "Do Not Disturb", offline: "Offline" };

// The line under a name: what they are playing, else emoji plus status.
function statusLine(person) {
  if (!person) return "";
  if (person.activity) return person.activity;
  return [person.status_emoji, person.custom_status].filter(Boolean).join(" ");
}

// "Back at 3:40 PM" (or "Back Thu at 9:00 AM") while a timed Do Not
// Disturb still runs; empty once it has passed or when there is no time.
function backLine(person, now = Date.now()) {
  const until = Number(person && person.presence_until);
  if (!person || person.presence !== "do_not_disturb" || !until || until * 1000 <= now) return "";
  const d = new Date(until * 1000);
  const time = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const sameDay = d.toDateString() === new Date(now).toDateString();
  return sameDay ? `Back at ${time}` : `Back ${d.toLocaleDateString([], { weekday: "short" })} at ${time}`;
}

// What goes under a name: when someone is on Do Not Disturb with a
// deadline, when they are back comes first, then whatever they are
// doing ("Back at 3:40 PM · in a meeting"). Otherwise the status line.
function subLine(person, now = Date.now()) {
  return [backLine(person, now), statusLine(person)].filter(Boolean).join(" · ");
}

// The presence as a line of text: "Do Not Disturb · back at 3:40 PM".
function presenceLabel(person, now = Date.now()) {
  const base = PRESENCE_LABELS[(person && person.presence) || "offline"] || "Offline";
  const back = backLine(person, now);
  return back ? `${base} · ${back[0].toLowerCase()}${back.slice(1)}` : base;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { PRESENCE_LABELS, statusLine, backLine, subLine, presenceLabel };
}
