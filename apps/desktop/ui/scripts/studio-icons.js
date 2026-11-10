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

const STUDIO_APP_ICONS = {
  phone: "phone", speaker: "volume-2", mic: "mic", micOff: "mic-off", headset: "headphones", headsetOff: "headphone-off",
  chatBubble: "message-circle", chat: "message-square-text", plus: "plus", forum: "messages-square", grid: "layout-grid", list: "list",
  screen: "monitor-up", screenOff: "monitor-x", hangup: "phone-off", signal: "bars", noise: "audio-lines", soundboard: "music",
  flag: "flag", more: "ellipsis", block: "ban", trash: "trash", eyeOff: "eye-off", thread: "message-square-text", lock: "lock", archive: "archive",
  reply: "reply", smilePlus: "smile-plus", play: "play", pause: "pause", send: "arrow-up", folder: "folder", camera: "video", cameraOff: "video-off",
  gear: "settings", pencil: "pencil", pointer: "mouse-pointer-2", eraser: "eraser", person: "user", friendAdd: "user-plus", serverAdd: "circle-plus",
  smile: "smile", fullscreen: "maximize", expand: "maximize-2", collapse: "minimize-2", calendar: "calendar-days", coffee: "heart-handshake",
  book: "scroll-text", chevronLeft: "chevron-left", chevronRight: "chevron-right", chevronDown: "chevron-down", roles: "tag", check: "check",
  poll: "chart-bar-big", close: "x", eye: "eye", pin: "pin", pinOutline: "pin", pinOff: "pin-off", megaphone: "megaphone", uploadFile: "file",
  photo: "image", movie: "film", gifBox: "gif", bellOff: "bell-off", people: "users", gamepad: "gamepad-2", paste: "clipboard", search: "search",
  clock: "clock", link: "link", download: "download", warning: "triangle-alert", openExternal: "external-link", bookmark: "bookmark",
  bookmarkOn: "bookmark", bell: "bell", stage: "podcast", hand: "hand", emojiSmile: "smile", emojiClock: "clock", emojiHeart: "heart",
  emojiFlag: "flag", globe: "globe",
};
const STUDIO_APP_ICONS_ON = new Set(["pin", "bookmarkOn"]);
const STUDIO_STATIC_ICONS = [
  ["M5 3.5h14A2.5", "message-circle"],
  ["M11 5h2v6h6v2", "plus"],
  ["M12 10.9c-.61", "compass"],
  ["M19.4 13a7.9", "settings"],
  ["M15 12a4 4 0 1 0-4-4 4 4 0 0 0 4 4zm-9-2", "user-plus"],
  ["M15.4 5.4 14 4l-8 8", "chevron-left"],
  ["M16 9V4h1c.55", "pin"],
  ["M17 3H7a2 2 0 0 0-2 2v16l7-3", "bookmark"],
  ["M16 11a3 3 0 1 0-3-3", "users"],
  ["M16 13v-2H7V8l-5 4", "log-out"],
  ["M12 12a4 4 0 1 0-4-4 4 4 0 0 0 4 4zm0 2c-2.7", "user"],
  ["M18.3 5.7 12 12", "x"],
  ["M18.4 4.2 12 10.6", "x"],
  ["M12 2a10 10 0 1 0 10 10A10 10 0 0 0 12 2zm0 18a8 8 0 1 1 8-8 8 8 0 0 1-8 8zm.5-13", "clock"],
  ["M12 2a10 10 0 1 0 10 10A10 10 0 0 0 12 2zm0 18a8 8 0 1 1 8-8 8 8 0 0 1-8 8zM8.5 11", "smile"],
  ["M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zm0 18a8 8 0 1 1 0-16", "smile"],
  ["M12 19V5M5.5 11.5", "arrow-up"],
];

function studioAppIconName(name, source) {
  if (typeof name !== "string" || !Object.prototype.hasOwnProperty.call(STUDIO_APP_ICONS, name)) return "";
  const mapped = STUDIO_APP_ICONS[name];
  return studioIconKnown(mapped, source) ? mapped : "";
}

function studioAppIconOn(name) {
  return STUDIO_APP_ICONS_ON.has(name);
}

function studioStaticIconName(d, source) {
  const drawn = String(d || "");
  const hit = STUDIO_STATIC_ICONS.find(([start]) => drawn.startsWith(start));
  return hit && studioIconKnown(hit[1], source) ? hit[1] : "";
}

function studioIconNode(name, size, on, doc) {
  const holder = (doc || document).createElement("template");
  holder.innerHTML = studioIconMarkup(name, on ? "ai on" : "ai");
  const svg = holder.content.firstElementChild;
  if (!svg) return null;
  svg.setAttribute("width", size);
  svg.setAttribute("height", size);
  svg.style.width = `${size}px`;
  svg.style.height = `${size}px`;
  return svg;
}

function swapStaticStudioIcons(root) {
  let swapped = 0;
  for (const old of root.querySelectorAll('svg[viewBox="0 0 24 24"]')) {
    if (old.childElementCount !== 1) continue;
    const name = studioStaticIconName(old.firstElementChild.getAttribute("d"));
    if (!name) continue;
    const svg = studioIconNode(name, Number(old.getAttribute("width")) || 20, false, old.ownerDocument);
    if (!svg) continue;
    old.replaceWith(svg);
    swapped += 1;
  }
  return swapped;
}

const StudioIcons = { markup: studioIconMarkup, paint: paintStudioIcons, known: studioIconKnown };

if (typeof document !== "undefined" && document.documentElement.dataset.look === "studio") swapStaticStudioIcons(document);

if (typeof module !== "undefined" && module.exports) {
  module.exports = { STUDIO_ICON_GLYPHS, STUDIO_APP_ICONS, STUDIO_STATIC_ICONS, StudioIcons, studioAppIconName, studioAppIconOn, studioStaticIconName, studioIconNode, swapStaticStudioIcons, studioIconKnown, studioIconIsGlyph, studioIconClasses, studioIconMarkup, paintStudioIcons };
}
