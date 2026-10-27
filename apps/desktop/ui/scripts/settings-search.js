const SETTINGS_SEARCH_LIMIT = 20;
const SEARCH_FIELD_WEIGHTS = { title: 4, keywords: 3, section: 2, pageTitle: 1 };

function normalizeSearchText(text) {
  return String(text || "")
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .trim();
}

function searchWords(text) {
  const normal = normalizeSearchText(text);
  return normal ? normal.split(" ") : [];
}

function bestFieldWeight(word, fields) {
  let best = 0;
  for (const [name, words] of Object.entries(fields)) {
    if (words.some(candidate => candidate.startsWith(word))) best = Math.max(best, SEARCH_FIELD_WEIGHTS[name]);
  }
  return best;
}

function scoreSettingEntry(entry, queryWords, phrase) {
  const fields = {
    title: searchWords(entry.title),
    keywords: searchWords(entry.keywords),
    section: searchWords(entry.section),
    pageTitle: searchWords(entry.pageTitle),
  };
  let score = 0;
  let matchedOwnText = false;
  for (const word of queryWords) {
    const weight = bestFieldWeight(word, fields);
    if (!weight) return 0;
    if (weight > SEARCH_FIELD_WEIGHTS.section || entry.kind === "page") matchedOwnText = true;
    score += weight;
  }
  if (!matchedOwnText) return 0;
  const title = normalizeSearchText(entry.title);
  if (title === phrase) score += 6;
  else if (title.startsWith(phrase)) score += 3;
  if (entry.kind === "page") score += 1;
  return score;
}

function searchSettings(entries, query, limit = SETTINGS_SEARCH_LIMIT) {
  const queryWords = searchWords(query);
  if (!queryWords.length) return [];
  const phrase = queryWords.join(" ");
  const seen = new Set();
  return entries
    .map((entry, order) => ({ entry, order, score: scoreSettingEntry(entry, queryWords, phrase) }))
    .filter(hit => hit.score > 0)
    .sort((a, b) => b.score - a.score || a.order - b.order)
    .map(hit => hit.entry)
    .filter(entry => {
      const key = `${entry.page}\n${normalizeSearchText(entry.title)}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .slice(0, limit);
}

function settingsResultPlace(entry) {
  if (entry.kind === "page") return "";
  return entry.section && normalizeSearchText(entry.section) !== normalizeSearchText(entry.title)
    ? `${entry.pageTitle} › ${entry.section}`
    : entry.pageTitle;
}

if (typeof module !== "undefined") module.exports = { normalizeSearchText, searchSettings, settingsResultPlace, SETTINGS_SEARCH_LIMIT };
