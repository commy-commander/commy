"use strict";

const CLEAR_ALL_WORD = "DELETE";

const USAGE_PARTS = [
  { id: "messages", get label() { return t("ui.messages-and-servers"); }, tone: "tx-red", cache: false },
  { id: "shared_files", get label() { return t("ui.shared-files"); }, tone: "tx-orange", cache: false },
  { id: "voices", get label() { return t("ui.text-to-speech-voices"); }, tone: "tx-yellow", cache: false },
  { id: "pictures", get label() { return t("ui.saved-profile-pictures"); }, tone: "tx-green", cache: true },
  { id: "web_cache", get label() { return t("ui.web-cache"); }, tone: "tx-teal", cache: true },
  { id: "logs", get label() { return t("ui.logs"); }, tone: "tx-blue", cache: false },
  { id: "other_accounts", get label() { return t("ui.other-accounts-on-this-computer"); }, tone: "tx-purple", cache: false },
  { id: "other", get label() { return t("ui.settings-keys-and-other-files"); }, tone: "tx-pink", cache: false },
];

const FOLDER_NOTES = {
  get default() { return t("ui.commy-s-standard-folder-on-this-computer"); },
  get chosen() { return t("ui.a-folder-you-chose"); },
  get overridden() { return t("ui.set-with-commy-data-dir-for-this-copy-of"); },
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
  const free = Number.isFinite(usage.free) ? t("ui.free-on-this-drive", { value: formatBytes(usage.free) }) : "";
  return { total, totalText: formatBytes(total), freeText: free, rows, cacheText: formatBytes(cache) };
}

function folderLabel(path) {
  const parts = String(path || "").split(/[\\/]+/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : "";
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
  const where = plan.to_default ? t("ui.commy-s-standard-folder") : plan.target;
  const lines = [
    t("ui.commy-will-close-copy-to-check-every", { value: formatBytes(plan.bytes), where }),
    t("ui.the-old-copy-is-removed-only-after-the"),
  ];
  if (inCall) lines.push(t("ui.you-ll-leave-the-voice-channel-you-re-in"));
  return { title: plan.to_default ? t("ui.move-back-to-the-standard-folder") : t("ui.move-your-data"), lines };
}

function describeClearedCache(cleared) {
  const now = Number(cleared.freed) || 0;
  const later = Number(cleared.on_restart) || 0;
  if (later > 0) return t("ui.cleared-another-clears-the-next-time", { value: formatBytes(now), value2: formatBytes(later) });
  return now > 0 ? t("ui.cleared", { value: formatBytes(now) }) : t("ui.the-cache-was-already-empty");
}

function clearAllConfirmed(typed) {
  return String(typed || "").trim() === CLEAR_ALL_WORD;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    CLEAR_ALL_WORD,
    USAGE_PARTS,
    formatBytes,
    folderLabel,
    describeUsage,
    describeFolder,
    describeMovePlan,
    describeClearedCache,
    clearAllConfirmed,
  };
}
