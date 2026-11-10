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
  module.exports = { STUDIO_DOCK_BARS, STUDIO_DOCK_FUSE_MS, STUDIO_DOCK_LINES, STUDIO_DOCK_QUALITY, studioDockQuality, studioDockWords, studioDockDesk, mountStudioDock, studioDockSound, createStudioDockFuse };
}
