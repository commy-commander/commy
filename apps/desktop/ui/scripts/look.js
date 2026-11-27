const LOOK_FX_KEY = "commy.fx";
const LOOK_ZOOM_KEY = "commy.zoom";
const LOOK_STACK_KEY = "commy.tallStack";
const LOOK_FIT = { slimWidth: 700, roomyWidth: 1180, roomyHeight: 760, phoneWidth: 550, stackHeight: 1800, tallRatio: 2, zoomMax: 8, tabletSide: 600, threePaneWidth: 1024 };
const LOOK_ROOT_DATA = ["look", "fit", "fx", "stack", "touch", "panes", "fold"];
const LOOK_FOLD_QUERIES = ["(horizontal-viewport-segments: 2)", "(vertical-viewport-segments: 2)"];
const LOOK_ZOOM_SETTLE = { everyMs: 50, steadyPolls: 3, maxPolls: 40, hops: 3 };

function tabletScreen(width, height) {
  return Math.min(width, height) >= LOOK_FIT.tabletSide;
}

function tabletFit(width, height) {
  if (width <= LOOK_FIT.slimWidth) return "slim";
  return width >= LOOK_FIT.roomyWidth && height >= LOOK_FIT.roomyHeight ? "roomy" : "compact";
}

function tabletPanes(width) {
  return width >= LOOK_FIT.threePaneWidth ? "3" : "2";
}

function foldPosture(segments) {
  if (!segments || segments.length !== 2) return null;
  const [a, b] = segments;
  if (a.y === b.y && a.height === b.height && a.x !== b.x) return "book";
  if (a.x === b.x && a.width === b.width && a.y !== b.y) return "tabletop";
  return null;
}

function windowFit(width, height, phoneApp = false, tablet = false) {
  if (phoneApp && tablet) return tabletFit(width, height);
  if (width <= LOOK_FIT.slimWidth || height >= width * LOOK_FIT.tallRatio) return "slim";
  if (phoneApp && height > width) return "slim";
  if (width >= LOOK_FIT.roomyWidth && height >= LOOK_FIT.roomyHeight) return "roomy";
  return "compact";
}

function windowZoom(width, height) {
  if (windowFit(width, height) !== "slim" || !(width > LOOK_FIT.phoneWidth)) return 1;
  return Math.min(LOOK_FIT.zoomMax, width / LOOK_FIT.phoneWidth);
}

function windowLayout({ width, height, zoom = 1, canZoom = false }) {
  const real = { width: width * zoom, height: height * zoom };
  const want = canZoom ? windowZoom(real.width, real.height) : 1;
  return { zoom: want, width: real.width / want, height: real.height / want };
}

function windowStack(width, height) {
  return windowFit(width, height) === "slim" && height / windowZoom(width, height) >= LOOK_FIT.stackHeight;
}

function stackChosen(stored) {
  return stored === "on";
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

function lookAttributes({ storedFx, storedStack = null, width, height, phoneApp, tablet = false, zoom = 1, segments = null }) {
  const real = { width: width * zoom, height: height * zoom };
  const fit = windowFit(real.width, real.height, phoneApp, tablet);
  const stack = !phoneApp && stackChosen(storedStack) && windowStack(real.width, real.height);
  const attrs = {
    look: "studio",
    fit,
    fx: resolveFx(storedFx, { fit, phoneApp }) === "lite" ? "lite" : null,
    stack: stack ? "on" : null,
  };
  if (phoneApp && tablet && fit !== "slim") {
    const fold = foldPosture(segments);
    Object.assign(attrs, { touch: "tablet", panes: fold === "book" ? "2" : tabletPanes(real.width) });
    if (fold) attrs.fold = fold;
  }
  return attrs;
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

function setRootData(root, name, value) {
  if (value && root.dataset[name] !== value) root.dataset[name] = value;
  else if (!value && name in root.dataset) delete root.dataset[name];
}

function applyLook() {
  const root = document.documentElement;
  fitWindowZoom();
  const attrs = lookAttributes({
    storedFx: storedLookValue(LOOK_FX_KEY),
    storedStack: storedLookValue(LOOK_STACK_KEY),
    width: window.innerWidth,
    height: window.innerHeight,
    phoneApp: root.classList.contains("mobile-app"),
    tablet: Boolean(window.screen) && tabletScreen(window.screen.width, window.screen.height),
    zoom: lookWindow.zoom,
    segments: window.viewport && window.viewport.segments,
  });
  for (const name of LOOK_ROOT_DATA) setRootData(root, name, attrs[name]);
  const media = attrs.look ? "all" : "not all";
  for (const sheet of document.querySelectorAll('link[id^="studio-"]')) if (sheet.media !== media) sheet.media = media;
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

function setLookStack(on) {
  try {
    window.localStorage.setItem(LOOK_STACK_KEY, on ? "on" : "off");
  } catch {
    return false;
  }
  applyLook();
  return stackChosen(storedLookValue(LOOK_STACK_KEY));
}

function lookStackChosen() {
  return stackChosen(storedLookValue(LOOK_STACK_KEY));
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
  if (typeof window.matchMedia === "function") for (const query of LOOK_FOLD_QUERIES) window.matchMedia(query).addEventListener("change", applyLook);
  let lookPhoneApp = document.documentElement.classList.contains("mobile-app");
  new MutationObserver(() => {
    const phoneApp = document.documentElement.classList.contains("mobile-app");
    if (phoneApp === lookPhoneApp) return;
    lookPhoneApp = phoneApp;
    applyLook();
  }).observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { LOOK_FX_KEY, LOOK_ZOOM_KEY, LOOK_STACK_KEY, LOOK_FIT, foldPosture, tabletScreen, tabletFit, tabletPanes, windowFit, windowZoom, windowLayout, windowStack, stackChosen, zoomHop, knownZoom, resolveFx, lookAttributes };
}
