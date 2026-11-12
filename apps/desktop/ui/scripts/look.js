const LOOK_KEY = "commy.look.2";
const LOOK_FX_KEY = "commy.fx";
const LOOK_DEFAULT = "studio";
const LOOK_ZOOM_KEY = "commy.zoom";
const LOOK_FIT = { slimWidth: 700, roomyWidth: 1180, roomyHeight: 760, stackWidth: 550, stackHeight: 1800, tallRatio: 2, zoomMax: 8 };
const LOOK_ZOOM_SETTLE = { everyMs: 50, steadyPolls: 3, maxPolls: 40, hops: 3 };

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

function windowFit(width, height, phoneApp = false) {
  if (width <= LOOK_FIT.slimWidth || height >= width * LOOK_FIT.tallRatio) return "slim";
  if (phoneApp && height > width) return "slim";
  if (width >= LOOK_FIT.roomyWidth && height >= LOOK_FIT.roomyHeight) return "roomy";
  return "compact";
}

function windowZoom(width, height) {
  if (windowFit(width, height) !== "slim" || !(width > LOOK_FIT.stackWidth)) return 1;
  return Math.min(LOOK_FIT.zoomMax, width / LOOK_FIT.stackWidth);
}

function windowLayout({ width, height, zoom = 1, canZoom = false }) {
  const real = { width: width * zoom, height: height * zoom };
  const want = canZoom ? windowZoom(real.width, real.height) : 1;
  return { zoom: want, width: real.width / want, height: real.height / want };
}

function windowStack(width, height) {
  return windowFit(width, height) === "slim" && height / windowZoom(width, height) >= LOOK_FIT.stackHeight;
}

function zoomHop(zoomNow, zoomWanted, hops) {
  if (Math.abs(zoomWanted - zoomNow) < 0.01) return "stay";
  return hops >= LOOK_ZOOM_SETTLE.hops ? "give-up" : "move";
}

function knownZoom(stored) {
  const zoom = Number(stored);
  return zoom >= 1 && zoom <= LOOK_FIT.zoomMax ? zoom : 1;
}

function resolveFx(stored, { fit, phoneApp }) {
  if (stored === "glass" || stored === "lite") return stored;
  return fit === "slim" || phoneApp ? "lite" : "glass";
}

function lookAttributes({ stored, search, storedFx, width, height, phoneApp, zoom = 1 }) {
  const look = resolveLook(stored, search);
  if (look !== "studio") return { look: null, fit: null, fx: null, stack: null };
  const real = { width: width * zoom, height: height * zoom };
  const fit = windowFit(real.width, real.height, phoneApp);
  const stack = !phoneApp && windowStack(real.width, real.height);
  return { look, fit, fx: resolveFx(storedFx, { fit, phoneApp }) === "lite" ? "lite" : null, stack: stack ? "on" : null };
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

const lookWindow = { zoom: 1, moving: false, broken: false, hops: 0, idle: 0 };

function lookWebview() {
  const tauri = window.__TAURI__;
  if (lookWindow.broken || document.documentElement.classList.contains("mobile-app") || !tauri || !tauri.webview || !tauri.webview.getCurrentWebview) return null;
  return tauri.webview.getCurrentWebview();
}

function rememberZoom(zoom) {
  lookWindow.zoom = zoom;
  try {
    window.sessionStorage.setItem(LOOK_ZOOM_KEY, String(zoom));
  } catch {
    return;
  }
}

function zoomFailed(view) {
  if (view && view.setZoom) view.setZoom(1).catch(() => {});
  lookWindow.broken = true;
  lookWindow.moving = false;
  rememberZoom(1);
  applyLook();
}

function setWindowZoom(view, want) {
  lookWindow.moving = true;
  const widthBefore = window.innerWidth;
  const moved = Math.abs(want - lookWindow.zoom) >= 0.01;
  let last = widthBefore;
  let steady = 0;
  let polls = 0;
  const poll = () => {
    polls += 1;
    steady = window.innerWidth === last ? steady + 1 : 0;
    last = window.innerWidth;
    if (steady < LOOK_ZOOM_SETTLE.steadyPolls && polls < LOOK_ZOOM_SETTLE.maxPolls) {
      setTimeout(poll, LOOK_ZOOM_SETTLE.everyMs);
      return;
    }
    lookWindow.idle = moved && last === widthBefore ? lookWindow.idle + 1 : 0;
    if (lookWindow.idle >= 2) {
      zoomFailed(view);
      return;
    }
    rememberZoom(want);
    lookWindow.moving = false;
    applyLook();
  };
  view.setZoom(want).then(() => setTimeout(poll, LOOK_ZOOM_SETTLE.everyMs), () => zoomFailed(null));
}

function fitWindowZoom() {
  const view = lookWebview();
  if (!view || lookWindow.moving) return;
  const want = windowLayout({ width: window.innerWidth, height: window.innerHeight, zoom: lookWindow.zoom, canZoom: true }).zoom;
  const hop = zoomHop(lookWindow.zoom, want, lookWindow.hops);
  if (hop === "stay") {
    lookWindow.hops = 0;
    return;
  }
  if (hop === "give-up") {
    zoomFailed(view);
    return;
  }
  lookWindow.hops += 1;
  setWindowZoom(view, want);
}

function applyLook() {
  const root = document.documentElement;
  fitWindowZoom();
  const attrs = lookAttributes({
    stored: storedLookValue(LOOK_KEY),
    search: window.location.search,
    storedFx: storedLookValue(LOOK_FX_KEY),
    width: window.innerWidth,
    height: window.innerHeight,
    phoneApp: root.classList.contains("mobile-app"),
    zoom: lookWindow.zoom,
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

function lookSearch(search) {
  const kept = String(search || "").replace(/^\?/, "").split("&").filter(part => part && !["studio", "classic"].includes(part.split("=")[0]));
  return kept.length ? "?" + kept.join("&") : "";
}

function chooseLook(look) {
  try {
    window.localStorage.setItem(LOOK_KEY, look === "studio" ? "studio" : "classic");
  } catch {
    return false;
  }
  const search = lookSearch(window.location.search);
  if (search === window.location.search) window.location.reload();
  else window.location.search = search;
  return true;
}

if (typeof document !== "undefined") {
  try {
    lookWindow.zoom = knownZoom(window.sessionStorage.getItem(LOOK_ZOOM_KEY));
  } catch {
    lookWindow.zoom = 1;
  }
  if (lookWindow.zoom !== 1 && lookWebview()) setWindowZoom(lookWebview(), lookWindow.zoom);
  else lookWindow.zoom = 1;
  applyLook();
  window.addEventListener("resize", applyLook);
  let lookPhoneApp = document.documentElement.classList.contains("mobile-app");
  new MutationObserver(() => {
    const phoneApp = document.documentElement.classList.contains("mobile-app");
    if (phoneApp === lookPhoneApp) return;
    lookPhoneApp = phoneApp;
    applyLook();
  }).observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { LOOK_KEY, LOOK_FX_KEY, LOOK_ZOOM_KEY, LOOK_DEFAULT, LOOK_FIT, lookFromSearch, resolveLook, windowFit, windowZoom, windowLayout, windowStack, zoomHop, knownZoom, resolveFx, lookAttributes, lookSearch };
}
