const STAGE_KEYS = ["stage-ink-point", "stage-ink-pen", "stage-ink-clear", "stage-grid", "stage-focus", "stage-fullscreen", "video-close"];
const PICKER_KEYS = ["lightbox-open", "lightbox-save", "search-newer", "search-older"];

function stageLiveWord(roomy) {
  return roomy ? "Live" : "LIVE";
}

function studioStageParts() {
  const roomy = document.documentElement.dataset.fit !== "slim";
  for (const key of STAGE_KEYS.map(id => document.getElementById(id)).filter(Boolean)) studioKeyDress(key, "key sm sq", roomy);
  for (const key of PICKER_KEYS.map(id => document.getElementById(id)).filter(Boolean)) studioKeyDress(key, "key sm", roomy);
  const close = document.getElementById("video-close");
  if (close) close.classList.toggle("danger", !roomy);
  const live = document.querySelector("#stage-view .stage-who > .live-badge");
  if (live) live.textContent = stageLiveWord(roomy);
}

function studioSearchStar() {
  for (const mark of document.querySelectorAll("#search-list > .search-empty > .pins-empty-mark")) {
    if (mark.querySelector(":scope > .sstar")) continue;
    const star = document.createElement("span");
    star.className = "star sstar";
    star.setAttribute("aria-hidden", "true");
    star.setAttribute("data-follow", "off");
    star.dataset.state = "quiet";
    mark.appendChild(star);
    if (typeof Star !== "undefined") Star.mount(star);
  }
}

function railServerId(key) {
  const match = /^server:(.+)$/.exec(String(key || ""));
  return match ? match[1] : null;
}

function studioRailTints(root) {
  if (typeof serverTint !== "function") return;
  for (const tile of root.querySelectorAll(".eb-rail-btn > .eb-rail-server")) {
    const id = railServerId(tile.parentElement.dataset.key);
    if (id && !tile.style.getPropertyValue("--server-tint")) tile.style.setProperty("--server-tint", serverTint(id));
  }
}

function studioFloat() {
  studioStageParts();
  new MutationObserver(studioStageParts).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit"] });
  const list = document.getElementById("search-list");
  if (list) new MutationObserver(studioSearchStar).observe(list, { childList: true });
  for (const id of ["composer-emoji-browser", "emoji-pop-browser"]) {
    const root = document.getElementById(id);
    if (root) new MutationObserver(() => studioRailTints(root)).observe(root, { childList: true, subtree: true });
  }
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { STAGE_KEYS, PICKER_KEYS, stageLiveWord, railServerId };
}
