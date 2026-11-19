const TYPING_TTL_MS = 8000;
const TYPING_SEND_EVERY_MS = 4000;
const MAX_TYPING_NAMES = 3;

class TypingTracker {
  constructor() {
    this.byChannel = new Map();
  }

  start(channel, user, now = Date.now()) {
    if (!channel || !user) return;
    if (!this.byChannel.has(channel)) this.byChannel.set(channel, new Map());
    this.byChannel.get(channel).set(user, now + TYPING_TTL_MS);
  }

  stop(channel, user) {
    const users = this.byChannel.get(channel);
    if (users) users.delete(user);
  }

  typers(channel, now = Date.now()) {
    const users = this.byChannel.get(channel);
    if (!users) return [];
    for (const [user, until] of users) if (until <= now) users.delete(user);
    return [...users.keys()];
  }

  anyone(now = Date.now()) {
    for (const channel of this.byChannel.keys()) if (this.typers(channel, now).length) return true;
    return false;
  }
}

function othersCount(count) {
  return t(count === 1 ? "ui.n.others.one" : "ui.n.others.many", { count: count });
}

function typingLine(names) {
  const list = (names || []).filter(Boolean);
  if (!list.length) return "";
  if (list.length === 1) return t("ui.is-typing", { value: list[0] });
  if (list.length === 2) return t("ui.and-are-typing", { value: list[0], value2: list[1] });
  if (list.length === MAX_TYPING_NAMES) return t("ui.and-are-typing-2", { value: list[0], value2: list[1], value3: list[2] });
  if (list.length <= 5) return t("ui.and-are-typing", { value: list.slice(0, MAX_TYPING_NAMES).join(", "), value2: othersCount(list.length - MAX_TYPING_NAMES) });
  return t("ui.several-people-are-typing");
}

class TypingSender {
  constructor(send) {
    this.send = send;
    this.lastSent = 0;
    this.channel = null;
  }

  typed(channel, text, now = Date.now()) {
    if (!channel || !String(text || "").trim()) return false;
    if (channel === this.channel && now - this.lastSent < TYPING_SEND_EVERY_MS) return false;
    this.channel = channel;
    this.lastSent = now;
    this.send(channel);
    return true;
  }

  sent() {
    this.lastSent = 0;
    this.channel = null;
  }
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { TYPING_TTL_MS, TYPING_SEND_EVERY_MS, TypingTracker, TypingSender, typingLine };
}
