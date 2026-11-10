const LOOK_KEY = "commy.look";
const LOOK_FX_KEY = "commy.fx";
const LOOK_DEFAULT = "classic";
const LOOK_FIT = { slimWidth: 700, roomyWidth: 1180, roomyHeight: 760 };

function lookFromSearch(search) {
  const words = String(search || "").replace(/^\?/, "").split("&").map(part => part.split("=")[0]);
  if (words.includes("studio")) return "studio";
  if (words.includes("classic")) return "classic";
  return null;
}

function resolveLook(stored, search) {
  const asked = lookFromSearch(search);
  if (asked) return asked;
  return stored === "studio" || stored === "classic" ? stored : LOOK_DEFAULT;
}

function windowFit(width, height) {
  if (width <= LOOK_FIT.slimWidth) return "slim";
  if (width >= LOOK_FIT.roomyWidth && height >= LOOK_FIT.roomyHeight) return "roomy";
  return "compact";
}

function resolveFx(stored, { fit, phoneApp }) {
  if (stored === "glass" || stored === "lite") return stored;
  return fit === "slim" || phoneApp ? "lite" : "glass";
}

function lookAttributes({ stored, search, storedFx, width, height, phoneApp }) {
  const look = resolveLook(stored, search);
  if (look !== "studio") return { look: null, fit: null, fx: null };
  const fit = windowFit(width, height);
  return { look, fit, fx: resolveFx(storedFx, { fit, phoneApp }) === "lite" ? "lite" : null };
}

function studioLook() {
  return typeof document !== "undefined" && document.documentElement.dataset.look === "studio";
}

function storedLookValue(key) {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function applyLook() {
  const root = document.documentElement;
  const attrs = lookAttributes({
    stored: storedLookValue(LOOK_KEY),
    search: window.location.search,
    storedFx: storedLookValue(LOOK_FX_KEY),
    width: window.innerWidth,
    height: window.innerHeight,
    phoneApp: root.classList.contains("mobile-app"),
  });
  for (const [name, value] of Object.entries(attrs)) {
    if (value) root.dataset[name] = value;
    else delete root.dataset[name];
  }
  for (const sheet of document.querySelectorAll('link[id^="studio-"]')) sheet.media = attrs.look ? "all" : "not all";
  return attrs;
}

function setLookFx(value) {
  const root = document.documentElement;
  try {
    window.localStorage.setItem(LOOK_FX_KEY, value);
    applyLook();
  } catch {
    if (value === "lite") root.dataset.fx = "lite";
    else delete root.dataset.fx;
  }
  return root.dataset.fx === "lite" ? "lite" : "glass";
}

if (typeof document !== "undefined") {
  applyLook();
  window.addEventListener("resize", applyLook);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { LOOK_KEY, LOOK_FX_KEY, LOOK_DEFAULT, LOOK_FIT, lookFromSearch, resolveLook, windowFit, resolveFx, lookAttributes };
}
