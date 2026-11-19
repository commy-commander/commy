const PRESENCE_LABELS = { get online() { return t("ui.online"); }, get idle() { return t("ui.idle"); }, get do_not_disturb() { return t("ui.do-not-disturb"); }, get offline() { return t("ui.offline-2"); } };

function shownActivity(activity) {
  const playing = /^Playing (.+)$/.exec(activity);
  return playing ? t("ui.playing-game", { game: playing[1] }) : activity;
}

function statusLine(person) {
  if (!person) return "";
  if (person.activity) return shownActivity(person.activity);
  return [person.status_emoji, person.custom_status].filter(Boolean).join(" ");
}

function backLine(person, now = Date.now()) {
  const until = Number(person && person.presence_until);
  if (!person || person.presence !== "do_not_disturb" || !until || until * 1000 <= now) return "";
  const d = new Date(until * 1000);
  const time = d.toLocaleTimeString(dateLocale(), { hour: "numeric", minute: "2-digit" });
  const sameDay = d.toDateString() === new Date(now).toDateString();
  return sameDay ? t("ui.back-at", { time }) : t("ui.back-at-2", { value: d.toLocaleDateString(dateLocale(), { weekday: "short" }), time });
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
  if (onPhone(person)) return t("ui.offline-gets-notifications-on-phone");
  const base = PRESENCE_LABELS[(person && person.presence) || "offline"] || t("ui.offline-2");
  const back = backLine(person, now);
  return back ? `${base} · ${back[0].toLowerCase()}${back.slice(1)}` : base;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { PRESENCE_LABELS, shownActivity, statusLine, backLine, subLine, presenceLabel, onPhone, presenceDot };
}
