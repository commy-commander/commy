const MAX_SAVED_MESSAGES = 500;
const REMINDER_CHECK_MS = 30_000;
const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const TOMORROW_HOUR = 9;

const REMINDER_PRESETS = [
  { id: "20m", get label() { return t("ui.in-20-minutes"); } },
  { id: "1h", get label() { return t("ui.in-1-hour"); } },
  { id: "3h", get label() { return t("ui.in-3-hours"); } },
  { id: "tomorrow", get label() { return t("ui.tomorrow-morning"); } },
  { id: "custom", get label() { return t("ui.pick-a-time"); } },
];

function reminderDue(preset, nowMs) {
  const now = Number.isFinite(nowMs) ? nowMs : Date.now();
  switch (preset) {
    case "20m": return now + 20 * MINUTE_MS;
    case "1h": return now + HOUR_MS;
    case "3h": return now + 3 * HOUR_MS;
    case "tomorrow": {
      const next = new Date(now);
      next.setDate(next.getDate() + 1);
      next.setHours(TOMORROW_HOUR, 0, 0, 0);
      return next.getTime();
    }
    default: return null;
  }
}

function parseReminderTime(text, nowMs) {
  const now = Number.isFinite(nowMs) ? nowMs : Date.now();
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})/.exec(String(text || "").trim());
  if (!match) return null;
  const [, y, mo, d, h, mi] = match.map(Number);
  const when = new Date(y, mo - 1, d, h, mi, 0, 0);
  if (Number.isNaN(when.getTime()) || when.getMonth() !== mo - 1 || when.getDate() !== d) return null;
  if (when.getTime() <= now) return null;
  return when.getTime();
}

function dueReminders(reminders, nowMs) {
  return (reminders || []).filter(r => r && Number(r.due) <= nowMs);
}

function sortReminders(reminders) {
  return [...(reminders || [])].sort((a, b) => Number(a.due) - Number(b.due) || String(a.id).localeCompare(String(b.id)));
}

function sortSaved(saved) {
  return [...(saved || [])].sort((a, b) => Number(b.saved_at) - Number(a.saved_at) || String(b.message).localeCompare(String(a.message)));
}

function messageEntry(message, context) {
  const files = ((message && message.attachments) || []).map(a => a.filename).filter(Boolean);
  const dm = context && context.dm ? context.dm : null;
  return {
    message: message.id,
    server: dm ? null : (context && context.server && context.server.id) || null,
    server_name: dm ? null : (context && context.server && context.server.name) || null,
    channel: dm ? null : (context && context.channel && context.channel.id) || null,
    channel_name: dm ? null : (context && context.channel && context.channel.name) || null,
    dm: dm ? dm.user : null,
    dm_name: dm ? dm.name : null,
    author_id: message.author_id,
    author_name: message.author_name,
    content: message.content || "",
    attachment: files[0] || null,
    created_at: message.created_at,
  };
}

function savedOrigin(entry) {
  if (!entry) return "";
  if (entry.dm) return t("ui.direct-message-with", { value: entry.dm_name || "someone" });
  const server = entry.server_name || "a server";
  return entry.channel_name ? `${server} · #${entry.channel_name}` : server;
}

function savedExcerpt(entry, plain) {
  const raw = (plain || (s => s))(entry && entry.content ? entry.content : "");
  const text = raw.replace(/(\*\*|__|~~|`)/g, "").replace(/^#{1,6}\s+/gm, "").replace(/\s+/g, " ").trim();
  if (text) return text;
  if (entry && entry.attachment) return t("ui.attachment", { attachment: entry.attachment });
  return t("ui.no-text");
}

function savedTarget(entry) {
  if (!entry) return null;
  if (entry.dm) return `dm:${entry.dm}`;
  if (entry.server && entry.channel) return `channel:${entry.server}:${entry.channel}`;
  return null;
}

function reminderLabel(dueMs, nowMs) {
  const diff = Number(dueMs) - nowMs;
  if (diff <= 0) return t("ui.due-now");
  const due = new Date(dueMs);
  const now = new Date(nowMs);
  const time = due.toLocaleTimeString(dateLocale(), { hour: "numeric", minute: "2-digit" });
  if (diff < HOUR_MS) return t("ui.in-min", { value: Math.max(1, Math.round(diff / MINUTE_MS)) });
  if (due.toDateString() === now.toDateString()) return t("ui.today-2", { time });
  const tomorrow = new Date(now);
  tomorrow.setDate(now.getDate() + 1);
  if (due.toDateString() === tomorrow.toDateString()) return t("ui.tomorrow", { time });
  return `${due.toLocaleDateString(dateLocale(), { month: "short", day: "numeric" })} ${time}`;
}

function reminderNotice(reminder) {
  const entry = reminder && reminder.entry;
  if (!entry) return { title: t("ui.reminder"), body: "" };
  const who = entry.author_name || t("ui.someone");
  const where = entry.dm ? "" : entry.channel_name ? ` in #${entry.channel_name}` : "";
  return { title: t("ui.reminder-2", { who, where }), body: savedExcerpt(entry).slice(0, 200) };
}

function savedButtonTitle(reminders, nowMs) {
  const upcoming = (reminders || []).filter(r => Number(r.due) > nowMs).length;
  if (!upcoming) return t("ui.saved-messages");
  return upcoming === 1 ? t("ui.saved-messages-1-reminder") : t("ui.saved-messages-reminders", { upcoming });
}

function savedSummary(saved, reminders, nowMs) {
  const messages = (saved || []).length;
  const upcoming = (reminders || []).filter(r => Number(r.due) > nowMs).length;
  if (!messages && !upcoming) return t("ui.nothing-saved-yet");
  const parts = [];
  if (messages) parts.push(t(messages === 1 ? "ui.n.messages.one" : "ui.n.messages.many", { count: messages }));
  if (upcoming) parts.push(t(upcoming === 1 ? "ui.n.reminders.one" : "ui.n.reminders.many", { count: upcoming }));
  return parts.join(" · ");
}

function toDateTimeLocal(ms) {
  const d = new Date(ms);
  const pad = n => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    MAX_SAVED_MESSAGES, REMINDER_CHECK_MS, REMINDER_PRESETS, reminderDue, parseReminderTime, dueReminders, sortReminders, sortSaved,
    messageEntry, savedOrigin, savedExcerpt, savedTarget, reminderLabel, reminderNotice, savedButtonTitle, savedSummary, toDateTimeLocal,
  };
}
