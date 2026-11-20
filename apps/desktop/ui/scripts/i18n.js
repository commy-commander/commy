const DEFAULT_LANGUAGE = "en";

const locales = {};
let currentLanguage = DEFAULT_LANGUAGE;
let currentPlatform = null;

function registerLocale(code, strings) {
  locales[code] = { ...(locales[code] || {}), ...strings };
}

function availableLanguages() {
  return Object.keys(locales).map(code => ({ code, label: locales[code]["language.name"] || code }));
}

function hasLanguage(code) {
  return Object.prototype.hasOwnProperty.call(locales, code);
}

function setLanguage(code) {
  currentLanguage = hasLanguage(code) ? code : DEFAULT_LANGUAGE;
  return currentLanguage;
}

function currentLanguageCode() {
  return currentLanguage;
}

function setPlatform(name) {
  currentPlatform = name || null;
  return currentPlatform;
}

function keysFor(key) {
  return currentPlatform ? [`${key}.${currentPlatform}`, key] : [key];
}

function dateLocale() {
  return currentLanguage === DEFAULT_LANGUAGE ? [] : currentLanguage;
}

function interpolate(template, vars) {
  return String(template).replace(/\{(\w+)\}/g, (whole, name) => {
    if (!vars || vars[name] === undefined || vars[name] === null) return whole;
    return String(vars[name]);
  });
}

function lookup(key, language) {
  const strings = locales[language];
  if (strings && Object.prototype.hasOwnProperty.call(strings, key)) return strings[key];
  return undefined;
}

function lookupFirst(keys, language) {
  for (const key of keys) {
    const found = lookup(key, language);
    if (found !== undefined) return found;
  }
  return undefined;
}

function t(key, vars, language = currentLanguage) {
  const keys = keysFor(key);
  const found = lookupFirst(keys, language) ?? lookupFirst(keys, DEFAULT_LANGUAGE);
  return interpolate(found === undefined ? key : found, vars);
}

let englishIndex = { language: null, size: 0, byShown: new Map() };

function englishFor(text) {
  if (currentLanguage === DEFAULT_LANGUAGE) return text;
  if (englishIndex.language !== currentLanguage || englishIndex.size !== Object.keys(locales[currentLanguage] || {}).length) {
    const strings = locales[currentLanguage] || {};
    const byShown = new Map();
    for (const [key, value] of Object.entries(strings)) {
      if (!byShown.has(value) && lookup(key, DEFAULT_LANGUAGE) !== undefined) byShown.set(value, lookup(key, DEFAULT_LANGUAGE));
    }
    englishIndex = { language: currentLanguage, size: Object.keys(strings).length, byShown };
  }
  return englishIndex.byShown.get(text) ?? text;
}

function sameText(a, b) {
  return String(a).replace(/\s+/g, " ").trim() === String(b).replace(/\s+/g, " ").trim();
}

function isShownTranslation(key, text) {
  return keysFor(key).some(each => Object.values(locales).some(strings => Object.prototype.hasOwnProperty.call(strings, each) && sameText(strings[each], text)));
}

function missingKeys(code, reference = DEFAULT_LANGUAGE) {
  const base = locales[reference] || {};
  const target = locales[code] || {};
  return Object.keys(base).filter(key => !Object.prototype.hasOwnProperty.call(target, key));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { DEFAULT_LANGUAGE, registerLocale, availableLanguages, hasLanguage, setLanguage, currentLanguageCode, setPlatform, dateLocale, interpolate, t, englishFor, isShownTranslation, missingKeys };
}
