"use strict";

const CLEAR_ALL_WORD = "DELETE";

const USAGE_PARTS = [
  { id: "messages", label: "Messages and servers", tone: "tx-red", cache: false },
  { id: "shared_files", label: "Files you share", tone: "tx-orange", cache: false },
  { id: "voices", label: "Text-to-speech voices", tone: "tx-yellow", cache: false },
  { id: "pictures", label: "Saved profile pictures", tone: "tx-green", cache: true },
  { id: "web_cache", label: "Web cache", tone: "tx-teal", cache: true },
  { id: "logs", label: "Logs", tone: "tx-blue", cache: false },
  { id: "other_accounts", label: "Other accounts on this computer", tone: "tx-purple", cache: false },
  { id: "other", label: "Settings, keys and other files", tone: "tx-pink", cache: false },
];

const FOLDER_NOTES = {
  default: "Commy's standard folder on this computer.",
  chosen: "A folder you chose.",
  overridden: "Set with COMMY_DATA_DIR for this copy of Commy, so it can't be moved from here.",
};

function formatBytes(bytes) {
  const value = Number(bytes) || 0;
  if (value < 1024) return `${value} bytes`;
  const units = ["KB", "MB", "GB", "TB"];
  let size = value / 1024;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size.toFixed(1)} ${units[unit]}`;
}

function describeUsage(usage) {
  const bytesOf = id => Number((usage.parts || []).find(part => part.id === id)?.bytes) || 0;
  const total = Math.max(Number(usage.total) || 0, USAGE_PARTS.reduce((sum, part) => sum + bytesOf(part.id), 0));
  const rows = USAGE_PARTS.map(part => {
    const bytes = bytesOf(part.id);
    return { ...part, bytes, size: formatBytes(bytes), share: total > 0 ? bytes / total : 0 };
  }).filter(row => row.bytes > 0);
  const cache = rows.filter(row => row.cache).reduce((sum, row) => sum + row.bytes, 0);
  const free = Number.isFinite(usage.free) ? `${formatBytes(usage.free)} free on this drive` : "";
  return { total, totalText: formatBytes(total), freeText: free, rows, cacheText: formatBytes(cache) };
}

function describeFolder(view) {
  const kind = FOLDER_NOTES[view.kind] ? view.kind : "default";
  return {
    path: view.folder || "",
    note: FOLDER_NOTES[kind],
    canMove: Boolean(view.can_move),
    canUseStandard: Boolean(view.can_move) && kind === "chosen",
    movePending: view.scheduled?.action === "move",
  };
}

function describeMovePlan(plan, { inCall = false } = {}) {
  const where = plan.to_default ? "Commy's standard folder" : plan.target;
  const lines = [
    `Commy will close, copy ${formatBytes(plan.bytes)} to ${where}, check every file, and open again from there.`,
    "The old copy is removed only after the new one checks out. If anything goes wrong, Commy keeps using the folder it has now.",
  ];
  if (inCall) lines.push("You'll leave the voice channel you're in.");
  return { title: plan.to_default ? "Move back to the standard folder?" : "Move your data?", lines };
}

function describeClearedCache(cleared) {
  const now = Number(cleared.freed) || 0;
  const later = Number(cleared.on_restart) || 0;
  if (later > 0) return `Cleared ${formatBytes(now)}. Another ${formatBytes(later)} clears the next time Commy starts.`;
  return now > 0 ? `Cleared ${formatBytes(now)}.` : "The cache was already empty.";
}

function clearAllConfirmed(typed) {
  return String(typed || "").trim() === CLEAR_ALL_WORD;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    CLEAR_ALL_WORD,
    USAGE_PARTS,
    formatBytes,
    describeUsage,
    describeFolder,
    describeMovePlan,
    describeClearedCache,
    clearAllConfirmed,
  };
}
