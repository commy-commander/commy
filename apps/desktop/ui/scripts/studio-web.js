const WEB_UNLOCK_DIALOG = "dialog slab dc";

const WEB_UNLOCK_KEYS = { "web-unlock-ask": "key primary", "web-unlock-code-use": "key" };

const CAM_FOCUS_KEY = "key sm sq";

function webUnlockStarState(waiting, locked) {
  if (waiting) return "waiting";
  return locked ? "unimpressed" : "idle";
}

function webDressPlan(has, added, wanted, roomy) {
  const mine = String(added || "").split(" ").filter(Boolean);
  if (!roomy) return { add: [], remove: mine, record: [] };
  const add = String(wanted || "").split(" ").filter(name => name && !has.includes(name));
  return { add, remove: [], record: [...new Set([...mine, ...add])] };
}

function studioWebDress(node, wanted, roomy) {
  const plan = webDressPlan([...node.classList], node.dataset.webDress, wanted, roomy);
  if (plan.add.length) node.classList.add(...plan.add);
  if (plan.remove.length) node.classList.remove(...plan.remove);
  const record = plan.record.join(" ");
  if (record && node.dataset.webDress !== record) node.dataset.webDress = record;
  if (!record && node.dataset.webDress !== undefined) delete node.dataset.webDress;
}

function studioWebRoomy() {
  return document.documentElement.dataset.fit !== "slim";
}

function studioWebUnlockStar(modal) {
  let star = modal.querySelector(":scope > .dstar[data-studio-web]");
  if (!star) {
    star = document.createElement("span");
    star.className = "star dstar";
    star.dataset.studioWeb = "";
    star.setAttribute("aria-hidden", "true");
    star.setAttribute("data-lift", "");
    star.dataset.state = "idle";
    modal.prepend(star);
    if (typeof Star !== "undefined") Star.mount(star);
  }
  const wait = modal.querySelector("#web-unlock-wait");
  const ask = modal.querySelector("#web-unlock-ask-row");
  const state = webUnlockStarState(Boolean(wait && !wait.classList.contains("hidden")), Boolean(ask && !ask.classList.contains("hidden")));
  if (state === "unimpressed" && star.getAttribute("data-follow") !== "off") star.setAttribute("data-follow", "off");
  if (state !== "unimpressed" && star.hasAttribute("data-follow")) star.removeAttribute("data-follow");
  if (star.dataset.state === state) return;
  if (typeof Star !== "undefined") Star.set(star, state);
  else star.dataset.state = state;
}

function studioWebUnlock(modal) {
  const roomy = studioWebRoomy();
  studioWebDress(modal, WEB_UNLOCK_DIALOG, roomy);
  for (const [id, classes] of Object.entries(WEB_UNLOCK_KEYS)) {
    const key = modal.querySelector(`#${id}`);
    if (key) studioWebDress(key, classes, roomy);
  }
  if (!roomy) {
    for (const node of modal.querySelectorAll("[data-studio-web]")) node.remove();
    return;
  }
  studioWebUnlockStar(modal);
  const error = modal.querySelector("#web-unlock-error");
  if (error && error.textContent.trim() && !error.querySelector(":scope > .i") && typeof studioIconMarkup === "function") {
    error.insertAdjacentHTML("afterbegin", studioIconMarkup("circle-alert"));
    error.firstElementChild.dataset.studioWeb = "";
  }
}

function signinGoBright(dressed, secondary) {
  return dressed && !secondary;
}

function studioWebSigninGo() {
  const go = document.getElementById("signin-go");
  if (!go || !studioWebRoomy()) return;
  const bright = signinGoBright(go.classList.contains("key"), go.classList.contains("btn-secondary"));
  if (go.classList.contains("primary") !== bright) go.classList.toggle("primary", bright);
}

function studioWebCamKey() {
  const key = document.getElementById("cam-grid-focus");
  if (!key) return;
  const roomy = studioWebRoomy();
  studioWebDress(key, CAM_FOCUS_KEY, roomy);
  if (roomy && !key.querySelector("svg") && typeof paintFocusButtons === "function") paintFocusButtons();
}

function studioWebWatchUnlock(modal) {
  if (modal.dataset.webWatched !== undefined) return;
  modal.dataset.webWatched = "";
  const again = () => studioWebUnlock(modal);
  const rows = new MutationObserver(again);
  for (const id of ["web-unlock-wait", "web-unlock-ask-row"]) {
    const node = modal.querySelector(`#${id}`);
    if (node) rows.observe(node, { attributes: true, attributeFilter: ["class"] });
  }
  const error = modal.querySelector("#web-unlock-error");
  if (error) rows.observe(error, { childList: true });
  again();
}

function studioWeb() {
  const findUnlock = () => {
    const modal = document.getElementById("modal-web-unlock");
    if (modal) studioWebWatchUnlock(modal);
    return modal;
  };
  const backdrop = document.getElementById("modal-backdrop");
  if (!findUnlock() && backdrop) {
    const waiting = new MutationObserver(() => {
      if (findUnlock()) waiting.disconnect();
    });
    waiting.observe(backdrop, { childList: true });
  }
  studioWebCamKey();
  const go = document.getElementById("signin-go");
  if (go) new MutationObserver(studioWebSigninGo).observe(go, { attributes: true, attributeFilter: ["class"] });
  studioWebSigninGo();
  new MutationObserver(() => {
    const modal = document.getElementById("modal-web-unlock");
    if (modal) studioWebUnlock(modal);
    studioWebCamKey();
    studioWebSigninGo();
  }).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit"] });
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { WEB_UNLOCK_DIALOG, WEB_UNLOCK_KEYS, CAM_FOCUS_KEY, webUnlockStarState, webDressPlan, signinGoBright };
}
