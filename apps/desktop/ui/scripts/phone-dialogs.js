const PHONE_DIALOG_HEADS = ["h2", ".sw-head", ".notify-head", ".catalog-head"];

const PHONE_DIALOG_SAFE = [/^(?!join-wait-).+-cancel$/, /-later$/, /-deny$/, /^create-server-back$/, /^signin-back$/, /-close$/, /-done$/];

const phoneDialogs = { held: false, backing: false, swipe: null, swipedAt: 0, moving: false, movingTimer: 0, waiting: false };

function phoneDialogGate(id, { signedIn = true, laterShown = true, acceptShown = false } = {}) {
  if (["modal-username", "modal-claim", "modal-call"].includes(id)) return true;
  if (id === "modal-signin") return !signedIn;
  if (id === "modal-server-welcome") return !laterShown;
  if (id === "modal-rules") return acceptShown;
  return false;
}

function phoneDialogSafeId(ids) {
  for (const pattern of PHONE_DIALOG_SAFE) {
    const found = ids.find(id => pattern.test(id));
    if (found) return found;
  }
  return null;
}

function phoneDialogShown() {
  if (typeof studioSlim !== "function" || !studioSlim()) return null;
  const backdrop = document.getElementById("modal-backdrop");
  if (!backdrop || backdrop.classList.contains("hidden")) return null;
  return backdrop.querySelector(":scope > .modal.dialog.slab:not(.hidden)");
}

function phoneDialogVisible(node) {
  return Boolean(node) && !node.classList.contains("hidden") && node.getClientRects().length > 0;
}

function phoneDialogIsGate(modal) {
  const shown = id => phoneDialogVisible(document.getElementById(id));
  return phoneDialogGate(modal.id, {
    signedIn: typeof account === "undefined" || Boolean(account.signed_in),
    laterShown: shown("sw-later"),
    acceptShown: shown("rules-accept"),
  });
}

function phoneDialogSafeKey(modal) {
  const keys = [...modal.querySelectorAll(":scope > .modal-actions > button, :scope > .signin-back")].filter(phoneDialogVisible);
  const id = phoneDialogSafeId(keys.map(key => key.id));
  return id ? keys.find(key => key.id === id) : null;
}

function phoneDialogSafe(modal) {
  const key = phoneDialogSafeKey(modal);
  if (key && !key.disabled) key.click();
  else if (!key) hideModals();
}

function phoneDialogHead(modal) {
  for (const node of modal.querySelectorAll(":scope > .pd-head")) node.classList.remove("pd-head");
  const first = [...modal.children].find(node => node.matches(":not(.modal-actions)") && getComputedStyle(node).display !== "none");
  if (first && PHONE_DIALOG_HEADS.some(head => first.matches(head))) first.classList.add("pd-head");
}

function phoneDialogFit(modal) {
  modal.classList.toggle("pd-scrolls", modal.scrollHeight > modal.clientHeight + 1);
}

function phoneDialogDress() {
  const modal = phoneDialogShown();
  for (const other of document.querySelectorAll(".modal.pd-gate, .modal.pd-scrolls")) {
    if (other !== modal) other.classList.remove("pd-gate", "pd-scrolls");
  }
  if (!modal) return;
  modal.classList.toggle("pd-gate", phoneDialogIsGate(modal));
  phoneDialogHead(modal);
  phoneDialogFit(modal);
}

function phoneDialogMoved() {
  phoneDialogs.moving = false;
  if (!phoneDialogs.waiting) return;
  phoneDialogs.waiting = false;
  phoneDialogStep();
}

function phoneDialogWatchHistory() {
  for (const name of ["back", "go"]) {
    const real = history[name].bind(history);
    history[name] = (...args) => {
      phoneDialogs.moving = true;
      clearTimeout(phoneDialogs.movingTimer);
      phoneDialogs.movingTimer = setTimeout(phoneDialogMoved, 600);
      return real(...args);
    };
  }
}

function phoneDialogStep() {
  if (phoneDialogs.backing || typeof layerStep !== "function") return;
  const open = Boolean(phoneDialogShown());
  const step = layerStep("ptDialog", open, phoneDialogs.held, history.state);
  if (step === "push" && phoneDialogs.moving) {
    phoneDialogs.waiting = true;
    return;
  }
  if (step === "push") history.pushState({ ...(history.state || { pane: phonePane() }), ptDialog: true }, "");
  phoneDialogs.held = open;
  if (step === "back") {
    phoneDialogs.backing = true;
    history.back();
  }
}

function phoneDialogSync() {
  phoneDialogDress();
  phoneDialogStep();
}

function phoneDialogOnBack() {
  const waited = phoneDialogs.waiting;
  phoneDialogs.waiting = false;
  phoneDialogs.moving = false;
  clearTimeout(phoneDialogs.movingTimer);
  if (phoneDialogs.backing) {
    phoneDialogs.backing = false;
    phoneDialogStep();
    return;
  }
  if (waited) {
    phoneDialogStep();
    return;
  }
  if (history.state && history.state.ptDialog) return;
  const modal = phoneDialogShown();
  if (!modal) {
    phoneDialogs.held = false;
    return;
  }
  if (modal.classList.contains("pd-gate")) {
    history.pushState({ ...(history.state || { pane: phonePane() }), ptDialog: true }, "");
    return;
  }
  phoneDialogs.held = false;
  phoneDialogSafe(modal);
  phoneDialogStep();
}

function phoneDialogScrim(e) {
  const modal = phoneDialogShown();
  if (!modal || e.target !== document.getElementById("modal-backdrop")) return;
  e.stopPropagation();
  e.preventDefault();
  if (e.type === "mousedown" && !modal.classList.contains("pd-gate")) phoneDialogSafe(modal);
}

function phoneDialogAtTop(modal, from) {
  for (let node = from; node && node !== modal.parentNode; node = node.parentNode) if (node.scrollTop > 0) return false;
  return true;
}

function phoneDialogTouchStart(e) {
  phoneDialogs.swipe = null;
  const modal = phoneDialogShown();
  if (!modal) return;
  e.stopPropagation();
  if (e.touches.length !== 1 || modal.classList.contains("pd-gate") || !modal.contains(e.target)) return;
  if (e.target.closest("input, textarea, select")) return;
  const head = e.touches[0].clientY - modal.getBoundingClientRect().top < 64;
  if (!head && !phoneDialogAtTop(modal, e.target)) return;
  phoneDialogs.swipe = { modal, x: e.touches[0].clientX, y: e.touches[0].clientY, axis: null, dy: 0 };
}

function phoneDialogTouchMove(e) {
  const swipe = phoneDialogs.swipe;
  if (!swipe || !e.touches[0]) return;
  const dx = e.touches[0].clientX - swipe.x;
  const dy = e.touches[0].clientY - swipe.y;
  if (!swipe.axis) {
    swipe.axis = swipeAxis(dx, dy);
    if (swipe.axis === "x" || (swipe.axis === "y" && dy < 0)) {
      phoneDialogs.swipe = null;
      return;
    }
    if (swipe.axis === "y") swipe.modal.classList.add("pt-dragging");
  }
  if (swipe.axis !== "y") return;
  swipe.dy = sheetPull(dy);
  phoneDialogs.swipedAt = Date.now();
  if (e.cancelable) e.preventDefault();
  swipe.modal.style.translate = `0 ${swipe.dy}px`;
}

function phoneDialogTouchEnd(cancelled) {
  const swipe = phoneDialogs.swipe;
  phoneDialogs.swipe = null;
  if (!swipe || swipe.axis !== "y") return;
  const modal = swipe.modal;
  modal.classList.remove("pt-dragging");
  if (cancelled || !sheetCloses(swipe.dy)) {
    modal.style.translate = "";
    return;
  }
  const calm = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  modal.style.transition = "translate 200ms var(--ease-out)";
  modal.style.translate = "0 calc(100% + 48px)";
  setTimeout(() => {
    modal.style.transition = "";
    modal.style.translate = "";
    if (phoneDialogShown() === modal) phoneDialogSafe(modal);
  }, calm ? 0 : 200);
}

function phoneDialogsWire() {
  const backdrop = document.getElementById("modal-backdrop");
  if (!backdrop) return;
  const sync = () => phoneDialogSync();
  new MutationObserver(sync).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit"] });
  new MutationObserver(sync).observe(backdrop, { attributes: true, attributeFilter: ["class"] });
  for (const modal of backdrop.querySelectorAll(":scope > .modal")) new MutationObserver(sync).observe(modal, { attributes: true, attributeFilter: ["class"] });
  new MutationObserver(() => {
    const modal = phoneDialogShown();
    if (modal) phoneDialogDress();
  }).observe(backdrop, { subtree: true, childList: true });
  if (typeof ResizeObserver === "function") {
    const sizes = new ResizeObserver(() => {
      const modal = phoneDialogShown();
      if (modal) phoneDialogFit(modal);
    });
    for (const modal of backdrop.querySelectorAll(":scope > .modal")) sizes.observe(modal);
  }
  phoneDialogWatchHistory();
  window.addEventListener("popstate", phoneDialogOnBack);
  document.addEventListener("mousedown", phoneDialogScrim, true);
  document.addEventListener("click", phoneDialogScrim, true);
  document.addEventListener("click", e => {
    if (!e.isTrusted || !phoneDialogShown() || Date.now() - phoneDialogs.swipedAt > 350) return;
    e.preventDefault();
    e.stopPropagation();
  }, true);
  backdrop.addEventListener("touchstart", phoneDialogTouchStart, { passive: true });
  backdrop.addEventListener("touchmove", phoneDialogTouchMove, { passive: false });
  backdrop.addEventListener("touchend", () => phoneDialogTouchEnd(false), { passive: true });
  backdrop.addEventListener("touchcancel", () => phoneDialogTouchEnd(true), { passive: true });
  sync();
}

if (typeof document !== "undefined" && typeof studioSlim === "function") phoneDialogsWire();

if (typeof module !== "undefined" && module.exports) {
  module.exports = { PHONE_DIALOG_HEADS, PHONE_DIALOG_SAFE, phoneDialogGate, phoneDialogSafeId };
}
