const STUDIO_DOCK_BARS = 12;
const STUDIO_DOCK_FUSE_MS = 3000;
const STUDIO_DOCK_LINES = {
  armed: ["Press again to leave", "Or press anything else to stay."],
  deafened: ["Deafened", "Peace and quiet. Suspicious."],
  muted: ["Muted", "Chew freely. Nobody hears you."],
};

const STUDIO_DOCK_QUALITY = {
  good: ["Good connection", "Voice is clear"],
  ok: ["Fair connection", "Voice may break up now and then"],
  bad: ["Poor connection", "Voice is breaking up"],
};

const STUDIO_DOCK_SIGNAL_BARS = [
  "M40,184a8,8,0,0,0-8,8v8a8,8,0,0,0,16,0v-8A8,8,0,0,0,40,184Z",
  "M80,144a8,8,0,0,0-8,8v48a8,8,0,0,0,16,0V152A8,8,0,0,0,80,144Z",
  "M120,104a8,8,0,0,0-8,8v88a8,8,0,0,0,16,0V112A8,8,0,0,0,120,104Z",
  "M168,72V200a8,8,0,0,1-16,0V72a8,8,0,0,1,16,0Z",
  "M200,24a8,8,0,0,0-8,8V200a8,8,0,0,0,16,0V32A8,8,0,0,0,200,24Z",
];
const STUDIO_DOCK_SIGNAL_LIT = { good: 5, ok: 3, bad: 1 };

function studioDockSignalMarkup() {
  const bars = STUDIO_DOCK_SIGNAL_BARS.map(d => `<path d="${d}"/>`).join("");
  return `<svg class="i ai vp-bars" viewBox="0 0 256 256" fill="currentColor" aria-hidden="true" focusable="false">${bars}</svg>`;
}

function studioDockSignal(doc, size) {
  const holder = doc.createElement("template");
  holder.innerHTML = studioDockSignalMarkup();
  const svg = holder.content.firstElementChild;
  svg.setAttribute("width", size);
  svg.setAttribute("height", size);
  svg.style.width = `${size}px`;
  svg.style.height = `${size}px`;
  return svg;
}

function studioDockQuality(quality) {
  const words = STUDIO_DOCK_QUALITY[quality] || STUDIO_DOCK_QUALITY.good;
  return { line: words[0], sub: words[1] };
}

function studioDockWords({ status, where, muted, deafened, armed }) {
  const state = armed ? "armed" : deafened ? "deafened" : muted ? "muted" : "live";
  const lines = STUDIO_DOCK_LINES[state] || [status || "", where || ""];
  return { state, line: lines[0], sub: lines[1] };
}

function studioDockDesk(doc) {
  const root = doc.documentElement;
  return root.dataset.look === "studio" && root.dataset.fit !== "slim";
}

function studioDockKey(doc, id, cls, title, icon, makeIcon) {
  const key = doc.createElement("button");
  key.type = "button";
  key.id = id;
  key.className = cls;
  key.title = title;
  key.setAttribute("aria-label", title);
  const art = makeIcon ? makeIcon(icon, 18) : null;
  if (art) key.appendChild(art);
  return key;
}

function mountStudioDock(doc, makeIcon) {
  if (doc.documentElement.dataset.look !== "studio") return null;
  const panel = doc.getElementById("voice-panel");
  const row = panel ? panel.querySelector(".vp-voice") : null;
  if (!row) return null;
  const present = row.querySelector(".vp-invite");
  if (present) return present;
  const hand = studioDockKey(doc, "vp-hand-key", "icon-btn vp-hand-key hidden", "Raise hand", "hand", makeIcon);
  hand.setAttribute("aria-pressed", "false");
  const invite = studioDockKey(doc, "vp-invite", "icon-btn vp-invite hidden", "Invite", "user-plus", makeIcon);
  const meter = doc.createElement("span");
  meter.className = "vp-meter";
  meter.setAttribute("aria-hidden", "true");
  for (let i = 0; i < STUDIO_DOCK_BARS; i += 1) meter.appendChild(doc.createElement("i"));
  const fuse = doc.createElement("i");
  fuse.className = "vp-fuse";
  fuse.setAttribute("aria-hidden", "true");
  row.prepend(hand);
  row.append(invite, meter);
  panel.prepend(fuse);
  return invite;
}

function studioDockSound(doc, heard) {
  const panel = doc.getElementById("voice-panel");
  if (!panel) return false;
  panel.classList.toggle("has-sound", Boolean(heard));
  return true;
}

function createStudioDockFuse(ms = STUDIO_DOCK_FUSE_MS, timers = null) {
  const wait = timers ? timers.set : (fn, t) => setTimeout(fn, t);
  const stop = timers ? timers.clear : id => clearTimeout(id);
  let pending = null;
  let onChange = () => {};
  const settle = () => {
    if (pending === null) return false;
    stop(pending);
    pending = null;
    onChange(false);
    return true;
  };
  return {
    armed: () => pending !== null,
    watch(fn) {
      onChange = fn;
    },
    cancel: settle,
    press() {
      if (settle()) return "leave";
      pending = wait(() => {
        pending = null;
        onChange(false);
      }, ms);
      onChange(true);
      return "armed";
    },
  };
}

if (typeof document !== "undefined") {
  mountStudioDock(document, typeof studioIconNode === "undefined" ? null : (name, size) => studioIconNode(name, size, false));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { STUDIO_DOCK_SIGNAL_BARS, STUDIO_DOCK_SIGNAL_LIT, studioDockSignalMarkup, studioDockSignal, STUDIO_DOCK_BARS, STUDIO_DOCK_FUSE_MS, STUDIO_DOCK_LINES, STUDIO_DOCK_QUALITY, studioDockQuality, studioDockWords, studioDockDesk, mountStudioDock, studioDockSound, createStudioDockFuse };
}
