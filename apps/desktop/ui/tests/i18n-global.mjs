import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const i18n = require("../scripts/i18n.js");
i18n.registerLocale("en", require("../scripts/locales/en.js"));
i18n.registerLocale("es", require("../scripts/locales/es.js"));
i18n.setLanguage("en");
Object.assign(globalThis, {
  t: i18n.t,
  englishFor: i18n.englishFor,
  isShownTranslation: i18n.isShownTranslation,
  currentLanguageCode: i18n.currentLanguageCode,
  dateLocale: i18n.dateLocale,
  DEFAULT_LANGUAGE: i18n.DEFAULT_LANGUAGE,
});

const english = require("../scripts/locales/en.js");
export function englishSource(src) {
  return src
    .replace(/ data-i18n(?:-[a-z-]+)?=(["'])[\w.-]+\1/g, "")
    .replace(/\$\{t\("([\w.-]+)"\)\}/g, (all, key) => (key in english ? english[key] : all))
    .replace(/\bt\("([\w.-]+)"\)/g, (all, key) => (key in english ? JSON.stringify(english[key]) : all))
    .replace(/get ("[^"]+"|\w+)\(\) \{ return ("(?:[^"\\]|\\.)*"); \}/g, "$1: $2");
}
