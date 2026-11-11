const STAGE_KEYS = ["stage-ink-point", "stage-ink-pen", "stage-ink-clear", "stage-grid", "stage-focus", "stage-fullscreen", "video-close"];

function stageLiveWord(roomy) {
  return roomy ? "Live" : "LIVE";
}

function studioStageParts() {
  const roomy = document.documentElement.dataset.fit !== "slim";
  for (const key of STAGE_KEYS.map(id => document.getElementById(id)).filter(Boolean)) studioKeyDress(key, "key sm sq", roomy);
  const close = document.getElementById("video-close");
  if (close) close.classList.toggle("danger", !roomy);
  const live = document.querySelector("#stage-view .stage-who > .live-badge");
  if (live) live.textContent = stageLiveWord(roomy);
}

function studioFloat() {
  studioStageParts();
  new MutationObserver(studioStageParts).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit"] });
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { STAGE_KEYS, stageLiveWord };
}
