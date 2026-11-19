function startingLanguage(raw) {
  try {
    const saved = JSON.parse(raw);
    return saved && typeof saved.language === "string" ? saved.language : DEFAULT_LANGUAGE;
  } catch {
    return DEFAULT_LANGUAGE;
  }
}

if (typeof document !== "undefined") {
  let raw = null;
  try { raw = localStorage.getItem("commy.accessibility"); } catch {}
  document.documentElement.lang = setLanguage(startingLanguage(raw));
}

if (typeof module !== "undefined" && module.exports) module.exports = { startingLanguage };
