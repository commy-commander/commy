const DEFAULT_LANGUAGE = "en";

const locales = {};
let currentLanguage = DEFAULT_LANGUAGE;

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

function t(key, vars, language = currentLanguage) {
  const found = lookup(key, language) ?? lookup(key, DEFAULT_LANGUAGE);
  return interpolate(found === undefined ? key : found, vars);
}

function missingKeys(code, reference = DEFAULT_LANGUAGE) {
  const base = locales[reference] || {};
  const target = locales[code] || {};
  return Object.keys(base).filter(key => !Object.prototype.hasOwnProperty.call(target, key));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { DEFAULT_LANGUAGE, registerLocale, availableLanguages, hasLanguage, setLanguage, currentLanguageCode, interpolate, t, missingKeys };
}
