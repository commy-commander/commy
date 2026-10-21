const PLAYING_PREFIX = "Playing ";
const MAX_STEAM_APP_ID = 0xffffffff;

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

function splitPlaying(members) {
  const since = m => Number(m.activity_since) || Number.MAX_SAFE_INTEGER;
  const playing = [];
  const rest = [];
  for (const m of members || []) (!m.bot && isShowingActivity(m) ? playing : rest).push(m);
  playing.sort((a, b) => since(a) - since(b) || String(a.name).localeCompare(String(b.name)));
  return { playing, rest };
}

function activityArt(person, details) {
  const own = person && person.activity_cover;
  return own || (details && details.cover) || null;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { gameFromActivity, steamStoreUrl, isShowingActivity, splitPlaying, activityArt };
}
