const DIRECTORY_CATEGORIES = [
  { id: "gaming", get label() { return t("ui.gaming"); } },
  { id: "music", get label() { return t("ui.music"); } },
  { id: "art", get label() { return t("ui.art"); } },
  { id: "tech", get label() { return t("ui.tech"); } },
  { id: "science", get label() { return t("ui.science"); } },
  { id: "education", get label() { return t("ui.education"); } },
  { id: "friends", get label() { return t("ui.friends-2"); } },
  { id: "languages", get label() { return t("ui.languages"); } },
  { id: "sports", get label() { return t("ui.sports"); } },
  { id: "other", get label() { return t("ui.other"); } },
];
const DIRECTORY_BUCKETS = [10, 25, 50, 100, 250, 500, 1000];
const DIRECTORY_DESCRIPTION_MAX = 300;
const DIRECTORY_QUERY_MAX = 80;
const DIRECTORY_EXCERPT_MAX = 160;

function directoryCategoryLabel(id) {
  const found = DIRECTORY_CATEGORIES.find(c => c.id === id);
  return found ? found.label : t("ui.other");
}

function memberBucket(count) {
  const n = Math.max(0, Math.floor(Number(count) || 0));
  let bucket = DIRECTORY_BUCKETS[0];
  for (const b of DIRECTORY_BUCKETS) if (n >= b) bucket = b;
  return bucket;
}

function memberBucketLabel(bucket) {
  const b = memberBucket(bucket);
  const top = DIRECTORY_BUCKETS[DIRECTORY_BUCKETS.length - 1];
  const shown = b >= top ? top.toLocaleString("en-US") : String(b);
  return t("ui.members-2", { shown });
}

function normalizeDirectorySearch(text) {
  return String(text || "").replace(/\s+/g, " ").trim().toLowerCase().slice(0, DIRECTORY_QUERY_MAX);
}

function directoryExcerpt(text, max = DIRECTORY_EXCERPT_MAX) {
  const flat = String(text || "").replace(/\s+/g, " ").trim();
  if (!flat) return t("ui.no-description-yet");
  if (flat.length <= max) return flat;
  const cut = flat.slice(0, max);
  const space = cut.lastIndexOf(" ");
  return `${(space > max / 2 ? cut.slice(0, space) : cut).replace(/[\s,;:.!?-]+$/, "")}…`;
}

function directoryDescriptionCounter(text) {
  const length = String(text || "").length;
  return { text: `${length} / ${DIRECTORY_DESCRIPTION_MAX}`, over: length > DIRECTORY_DESCRIPTION_MAX };
}

function discoverySettingsState({ server, listing }) {
  const canManage = Boolean(server && server.can_manage_server);
  const isPublic = Boolean(server && server.public);
  const listed = Boolean(listing);
  let hint = "";
  if (!canManage) hint = t("ui.only-members-with-manage-server-can-list");
  else if (!isPublic && listed) hint = t("ui.still-listed-in-discover-unlist-it-or-it");
  else if (!isPublic) hint = t("ui.turn-on-public-server-above-to-list-it");
  else if (listed) hint = t("ui.listed-in-discover-anyone-browsing-can");
  else hint = t("ui.listing-shares-the-server-name-your");
  return { enabled: canManage && isPublic, listed, canUnlist: canManage && listed, hint };
}

function discoverEmptyView({ loading = false, error = "", unavailable = false, host = "", filtered = false } = {}) {
  const where = host ? t("ui.your-account-server", { host }) : "Your account server";
  if (loading) return { icon: "search", title: t("ui.looking-for-servers"), text: t("ui.loading-the-public-server-list"), retry: false };
  if (unavailable) {
    return { icon: "clock", title: t("ui.discover-is-not-available-yet"), text: t("ui.does-not-list-public-servers-yet-check", { where }), retry: true };
  }
  if (error) return { icon: "warning", title: t("ui.could-not-load-discover"), text: error, retry: true };
  if (filtered) return { icon: "search", title: t("ui.no-servers-found"), text: t("ui.try-another-word-or-category"), retry: false };
  return { icon: "search", title: t("ui.no-public-servers-yet"), text: t("ui.be-the-first-list-your-own-public-server"), retry: false };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    DIRECTORY_CATEGORIES, DIRECTORY_BUCKETS, DIRECTORY_DESCRIPTION_MAX, directoryCategoryLabel, memberBucket, memberBucketLabel,
    normalizeDirectorySearch, directoryExcerpt, directoryDescriptionCounter, discoverySettingsState, discoverEmptyView,
  };
}
