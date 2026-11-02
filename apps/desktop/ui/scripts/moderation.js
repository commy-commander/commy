const BAN_LENGTHS = [
  { id: "1h", label: "1 hour", hours: 1 },
  { id: "5h", label: "5 hours", hours: 5 },
  { id: "1d", label: "1 day", hours: 24 },
  { id: "3d", label: "3 days", hours: 72 },
  { id: "1w", label: "1 week", hours: 168 },
  { id: "2w", label: "2 weeks", hours: 336 },
  { id: "1mo", label: "1 month", hours: 720 },
  { id: "3mo", label: "3 months", hours: 2160 },
  { id: "forever", label: "Until unbanned", hours: null },
];

const UPLOAD_LIMITS_MB = [8, 25, 50, 100, 250, 500, 1024, 2048];
const BYTES_PER_MB = 1024 * 1024;

function uploadLimitLabel(mb) {
  if (!mb) return "Up to 4 GB";
  return mb >= 1024 && mb % 1024 === 0 ? `${mb / 1024} GB` : `${mb} MB`;
}

function uploadRefusal(server, sizeBytes) {
  const mb = server && Number(server.upload_limit_mb);
  if (!mb || Number(sizeBytes) <= mb * BYTES_PER_MB) return null;
  return `This server allows files up to ${uploadLimitLabel(mb)}`;
}

function spanWords(ms) {
  const minutes = Math.max(1, Math.round(Number(ms) / 60000));
  const units = [["week", 7 * 1440], ["day", 1440], ["hour", 60], ["minute", 1]];
  const [name, size] = units.find(([, size]) => minutes >= size);
  const n = Math.floor(minutes / size);
  return `${n} ${name}${n === 1 ? "" : "s"}`;
}

function sinceWords(at, now) {
  if (!at) return null;
  const ms = Number(now) - Number(at);
  return ms < 60000 ? "just now" : `${spanWords(ms)} ago`;
}

function banLine(ban, now = Date.now()) {
  const who = ban.by_name ? `Banned by ${ban.by_name}` : "Banned";
  const when = sinceWords(ban.at, now);
  const end = ban.until ? `ends in ${spanWords(Math.max(0, ban.until - now))}` : "until unbanned";
  return `${who}${when ? ` ${when}` : ""}, ${end}`;
}

function kickLine(kick, now = Date.now()) {
  const when = sinceWords(kick.at, now);
  return `Kicked by ${kick.by_name}${when ? ` ${when}` : ""}`;
}

function banChoiceText(member, serverName, hours) {
  const length = BAN_LENGTHS.find(b => b.hours === hours);
  if (!hours || !length) return `${member} is removed from ${serverName} and cannot rejoin until you unban them.`;
  return `${member} is removed from ${serverName} and can rejoin with an invite after ${length.label}.`;
}

function canSeeRemoved(server) {
  return Boolean(server && (server.can_ban || server.can_kick));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    BAN_LENGTHS, UPLOAD_LIMITS_MB, BYTES_PER_MB, uploadLimitLabel, uploadRefusal, spanWords, sinceWords, banLine, kickLine, banChoiceText, canSeeRemoved,
  };
}
