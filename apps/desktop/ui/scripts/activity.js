const PLAYING_PREFIX = "Playing ";
const MAX_STEAM_APP_ID = 0xffffffff;
const PLAYING_NOW_LIMIT = 4;

function gameFromActivity(activity) {
  const text = typeof activity === "string" ? activity : "";
  if (!text.startsWith(PLAYING_PREFIX)) return null;
  const name = text.slice(PLAYING_PREFIX.length).trim();
  return name || null;
}

function steamStoreUrl(appId) {
  const id = Number(appId);
  if (!Number.isInteger(id) || id <= 0 || id > MAX_STEAM_APP_ID) return null;
  return `https://store.steampowered.com/app/${id}/`;
}

function isShowingActivity(person) {
  return !!person && person.presence !== "offline" && person.presence !== "invisible" && !!gameFromActivity(person.activity);
}

function playingNow(members, meId, limit = PLAYING_NOW_LIMIT) {
  const since = m => Number(m.activity_since) || Number.MAX_SAFE_INTEGER;
  return (members || [])
    .filter(m => m.id !== meId && !m.bot && isShowingActivity(m))
    .sort((a, b) => since(a) - since(b) || String(a.name).localeCompare(String(b.name)))
    .slice(0, limit);
}

function activityArt(person, details) {
  const own = person && person.activity_cover;
  return own || (details && details.cover) || null;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { gameFromActivity, steamStoreUrl, isShowingActivity, playingNow, activityArt, PLAYING_NOW_LIMIT };
}
