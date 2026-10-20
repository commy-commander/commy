const ACCESSIBILITY_KEY = "commy.accessibility";

const THEMES = ["dark", "light", "system"];

const SATURATION_MAX = 100;

const ACCESSIBILITY_DEFAULTS = Object.freeze({
  reducedMotion: null,
  saturation: SATURATION_MAX,
  highContrast: false,
  theme: "dark",
  language: "en",
});

function clampSaturation(value) {
  const n = Number(value);
  if (!Number.isFinite(n)) return SATURATION_MAX;
  return Math.min(SATURATION_MAX, Math.max(0, Math.round(n)));
}

function readAccessibility(raw, languages = ["en"]) {
  let parsed = {};
  try {
    parsed = raw ? JSON.parse(raw) : {};
  } catch {
    parsed = {};
  }
  if (!parsed || typeof parsed !== "object") parsed = {};
  return {
    reducedMotion: typeof parsed.reducedMotion === "boolean" ? parsed.reducedMotion : null,
    saturation: clampSaturation(parsed.saturation ?? SATURATION_MAX),
    highContrast: parsed.highContrast === true,
    theme: THEMES.includes(parsed.theme) ? parsed.theme : ACCESSIBILITY_DEFAULTS.theme,
    language: languages.includes(parsed.language) ? parsed.language : ACCESSIBILITY_DEFAULTS.language,
  };
}

function writeAccessibility(prefs) {
  const clean = readAccessibility(JSON.stringify(prefs), [prefs.language]);
  return JSON.stringify(clean);
}

function resolveAccessibility(prefs, os = {}) {
  const osReducedMotion = Boolean(os.reducedMotion);
  const osDark = os.dark !== false;
  return {
    reducedMotion: prefs.reducedMotion === null || prefs.reducedMotion === undefined ? osReducedMotion : prefs.reducedMotion,
    followsOsMotion: prefs.reducedMotion === null || prefs.reducedMotion === undefined,
    theme: prefs.theme === "system" ? (osDark ? "dark" : "light") : prefs.theme,
    saturation: clampSaturation(prefs.saturation),
    highContrast: Boolean(prefs.highContrast),
    language: prefs.language,
  };
}

function accessibilityAttributes(resolved) {
  return {
    "data-theme": resolved.theme,
    "data-reduced-motion": resolved.reducedMotion ? "" : null,
    "data-high-contrast": resolved.highContrast ? "" : null,
    "data-saturation": resolved.saturation < SATURATION_MAX ? String(resolved.saturation) : null,
  };
}

function saturationCss(resolved) {
  return `${clampSaturation(resolved.saturation)}%`;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { ACCESSIBILITY_KEY, THEMES, SATURATION_MAX, ACCESSIBILITY_DEFAULTS, clampSaturation, readAccessibility, writeAccessibility, resolveAccessibility, accessibilityAttributes, saturationCss };
}
