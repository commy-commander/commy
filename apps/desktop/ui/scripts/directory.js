const DIRECTORY_CATEGORIES = [
  { id: "gaming", label: "Gaming" },
  { id: "music", label: "Music" },
  { id: "art", label: "Art" },
  { id: "tech", label: "Tech" },
  { id: "science", label: "Science" },
  { id: "education", label: "Education" },
  { id: "friends", label: "Friends" },
  { id: "languages", label: "Languages" },
  { id: "sports", label: "Sports" },
  { id: "other", label: "Other" },
];
const DIRECTORY_BUCKETS = [10, 25, 50, 100, 250, 500, 1000];
const DIRECTORY_DESCRIPTION_MAX = 300;
const DIRECTORY_QUERY_MAX = 80;
const DIRECTORY_EXCERPT_MAX = 160;

function directoryCategoryLabel(id) {
  const found = DIRECTORY_CATEGORIES.find(c => c.id === id);
  return found ? found.label : "Other";
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
  return `${shown}+ members`;
}

function normalizeDirectorySearch(text) {
  return String(text || "").replace(/\s+/g, " ").trim().toLowerCase().slice(0, DIRECTORY_QUERY_MAX);
}

function directoryExcerpt(text, max = DIRECTORY_EXCERPT_MAX) {
  const flat = String(text || "").replace(/\s+/g, " ").trim();
  if (!flat) return "No description yet.";
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
  if (!canManage) hint = "Only members with Manage Server can list this server.";
  else if (!isPublic) hint = "Turn on Public server above to list it in Discover.";
  else if (listed) hint = "Listed in Discover. Anyone browsing can join with a link that never expires.";
  else hint = "Listing shares the server name, your description, the category and a rounded member count with your account server.";
  return { enabled: canManage && isPublic, listed, hint };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    DIRECTORY_CATEGORIES, DIRECTORY_BUCKETS, DIRECTORY_DESCRIPTION_MAX, directoryCategoryLabel, memberBucket, memberBucketLabel,
    normalizeDirectorySearch, directoryExcerpt, directoryDescriptionCounter, discoverySettingsState,
  };
}
