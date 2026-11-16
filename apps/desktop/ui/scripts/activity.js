const PLAYING_PREFIX = "Playing ";
const MAX_STEAM_APP_ID = 0xffffffff;

const GAME_BUILD_WORDS = new Set(["shipping", "win64", "win32", "wingdk", "x64", "x86", "dx11", "dx12", "d3d11", "d3d12", "vulkan", "launcher", "client"]);
const GLUED_GAME_BUILD_WORDS = ["Shipping", "Launcher", "Client"];
const NAME_SEPARATORS = /[_\- ]+/;

function stripGluedBuildWord(text) {
  for (const suffix of GLUED_GAME_BUILD_WORDS) {
    if (!text.endsWith(suffix)) continue;
    const stem = text.slice(0, -suffix.length);
    const lastWord = stem.split(NAME_SEPARATORS).pop();
    if (/[a-z0-9]$/.test(stem) && lastWord.length > 1) return stem;
  }
  return text;
}

function tidyGameName(name) {
  const words = String(name || "").trim().split(/([_\- ]+)/).filter(Boolean);
  while (words.length > 2 && NAME_SEPARATORS.test(words[words.length - 2]) && GAME_BUILD_WORDS.has(words[words.length - 1].toLowerCase())) {
    words.splice(-2, 2);
  }
  if (words.length && NAME_SEPARATORS.test(words[words.length - 1])) words.pop();
  return stripGluedBuildWord(words.join(""));
}

function gameFromActivity(activity) {
  const text = typeof activity === "string" ? activity : "";
  if (!text.startsWith(PLAYING_PREFIX)) return null;
  const name = tidyGameName(text.slice(PLAYING_PREFIX.length));
  return name || null;
}

function shownActivity(activity) {
  const game = gameFromActivity(activity);
  return game ? `${PLAYING_PREFIX}${game}` : activity;
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
  module.exports = { tidyGameName, gameFromActivity, shownActivity, steamStoreUrl, isShowingActivity, splitPlaying, activityArt };
}
