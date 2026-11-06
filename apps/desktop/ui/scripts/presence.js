const PRESENCE_LABELS = { online: "Online", idle: "Idle", do_not_disturb: "Do Not Disturb", offline: "Offline" };

function statusLine(person) {
  if (!person) return "";
  if (person.activity) return person.activity;
  return [person.status_emoji, person.custom_status].filter(Boolean).join(" ");
}

function backLine(person, now = Date.now()) {
  const until = Number(person && person.presence_until);
  if (!person || person.presence !== "do_not_disturb" || !until || until * 1000 <= now) return "";
  const d = new Date(until * 1000);
  const time = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const sameDay = d.toDateString() === new Date(now).toDateString();
  return sameDay ? `Back at ${time}` : `Back ${d.toLocaleDateString([], { weekday: "short" })} at ${time}`;
}

function subLine(person, now = Date.now()) {
  return [backLine(person, now), statusLine(person)].filter(Boolean).join(" · ");
}

function onPhone(person) {
  const presence = (person && person.presence) || "offline";
  return Boolean(person && person.phone) && (presence === "offline" || presence === "invisible");
}

function presenceDot(person) {
  const presence = (person && person.presence) || "offline";
  const shown = presence === "invisible" ? "offline" : presence;
  return onPhone(person) ? `${shown} phone` : shown;
}

function presenceLabel(person, now = Date.now()) {
  if (onPhone(person)) return "Offline · gets notifications on phone";
  const base = PRESENCE_LABELS[(person && person.presence) || "offline"] || "Offline";
  const back = backLine(person, now);
  return back ? `${base} · ${back[0].toLowerCase()}${back.slice(1)}` : base;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { PRESENCE_LABELS, statusLine, backLine, subLine, presenceLabel, onPhone, presenceDot };
}
