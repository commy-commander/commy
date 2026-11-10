const STUDIO_ICON_VIEWBOX = "0 0 256 256";
const STUDIO_ICON_REST = 0;
const STUDIO_ICON_ON = 1;
const STUDIO_ICON_NAME = /^[a-z0-9-]+$/;
const STUDIO_ICON_EXTRA_CLASS = /^[A-Za-z0-9_-]+$/;
const STUDIO_ICON_GLYPHS = new Set([
  "plus", "x", "minus", "check", "check-check", "arrow-up", "arrow-down", "arrow-left", "arrow-right",
  "chevron-up", "chevron-down", "chevron-left", "chevron-right", "ellipsis", "hash", "at-sign", "a-large-small",
  "bars", "corner-down-left", "corner-down-right", "maximize",
]);

function studioIconPaths(source) {
  if (source) return source;
  return typeof STUDIO_ICON_PATHS === "undefined" ? {} : STUDIO_ICON_PATHS;
}

function studioIconKnown(name, source) {
  const paths = studioIconPaths(source);
  return typeof name === "string" && STUDIO_ICON_NAME.test(name) && Object.prototype.hasOwnProperty.call(paths, name);
}

function studioIconIsGlyph(name) {
  return STUDIO_ICON_GLYPHS.has(name);
}

function studioIconClasses(name, extraClasses) {
  const extra = String(extraClasses || "").split(/\s+/).filter(cls => STUDIO_ICON_EXTRA_CLASS.test(cls) && cls !== "i" && cls !== "i-g");
  return ["i", ...(studioIconIsGlyph(name) ? ["i-g"] : []), ...extra].join(" ");
}

function studioIconMarkup(name, extraClasses, source) {
  if (!studioIconKnown(name, source)) return "";
  const pair = studioIconPaths(source)[name];
  const drawing = studioIconIsGlyph(name)
    ? pair[STUDIO_ICON_REST]
    : `<g class="i-rest">${pair[STUDIO_ICON_REST]}</g><g class="i-on">${pair[STUDIO_ICON_ON]}</g>`;
  return `<svg class="${studioIconClasses(name, extraClasses)}" data-icon="${name}" viewBox="${STUDIO_ICON_VIEWBOX}" aria-hidden="true" focusable="false">${drawing}</svg>`;
}

function paintStudioIcons(root, source) {
  const scope = root || (typeof document === "undefined" ? null : document);
  if (!scope || typeof scope.querySelectorAll !== "function") return 0;
  let painted = 0;
  for (const placeholder of scope.querySelectorAll("i[data-i]")) {
    const markup = studioIconMarkup(placeholder.getAttribute("data-i"), placeholder.className, source);
    if (!markup) continue;
    placeholder.outerHTML = markup;
    painted += 1;
  }
  return painted;
}

const StudioIcons = { markup: studioIconMarkup, paint: paintStudioIcons, known: studioIconKnown };

if (typeof module !== "undefined" && module.exports) {
  module.exports = { STUDIO_ICON_GLYPHS, StudioIcons, studioIconKnown, studioIconIsGlyph, studioIconClasses, studioIconMarkup, paintStudioIcons };
}
