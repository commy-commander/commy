const DISCOVER_KEY_IDS = ["discover-more", "discover-retry"];
const DISCOVER_STAR_ICONS = ["clock", "triangle-alert"];
const DISCOVER_ERROR_ICONS = ["triangle-alert"];

function discoverEmptyLook(icon) {
  const name = String(icon || "");
  return {
    art: DISCOVER_STAR_ICONS.includes(name) ? "star" : "well",
    tone: DISCOVER_ERROR_ICONS.includes(name) ? "error" : "",
  };
}

function discoverKeyNames(roomy) {
  return { key: Boolean(roomy), sm: Boolean(roomy) };
}

function studioDiscoverFlip(node, name, on) {
  if (node.classList.contains(name) !== on) node.classList.toggle(name, on);
}

function studioDiscoverKeys(roomy) {
  const keys = [...document.querySelectorAll("#discover-grid .discover-card-foot > button")];
  for (const id of DISCOVER_KEY_IDS) {
    const key = document.getElementById(id);
    if (key) keys.push(key);
  }
  for (const key of keys) {
    for (const [name, on] of Object.entries(discoverKeyNames(roomy))) studioDiscoverFlip(key, name, on);
  }
}

function studioDiscoverStar(empty) {
  let star = empty.querySelector(":scope > .discover-star");
  if (star) return star;
  star = document.createElement("span");
  star.className = "star discover-star";
  star.setAttribute("aria-hidden", "true");
  star.dataset.state = "quiet";
  star.dataset.follow = "off";
  const art = document.getElementById("discover-empty-art");
  if (art) art.before(star);
  else empty.prepend(star);
  if (typeof Star !== "undefined") Star.mount(star);
  return star;
}

function studioDiscoverEmpty() {
  const empty = document.getElementById("discover-empty");
  if (!empty) return;
  studioDiscoverStar(empty);
  const svg = empty.querySelector("#discover-empty-art > svg");
  const look = discoverEmptyLook(svg ? svg.getAttribute("data-icon") : "");
  if (empty.dataset.art !== look.art) empty.dataset.art = look.art;
  if (look.tone) {
    if (empty.dataset.tone !== look.tone) empty.dataset.tone = look.tone;
  } else if (empty.hasAttribute("data-tone")) {
    empty.removeAttribute("data-tone");
  }
}

function studioDiscoverDress() {
  studioDiscoverKeys(document.documentElement.dataset.fit !== "slim");
  studioDiscoverEmpty();
}

function studioDiscover() {
  if (studioDiscover.watching) return;
  const grid = document.getElementById("discover-grid");
  const art = document.getElementById("discover-empty-art");
  if (!grid || !art || typeof MutationObserver === "undefined") return;
  studioDiscover.watching = true;
  const watch = new MutationObserver(studioDiscoverDress);
  watch.observe(grid, { childList: true });
  watch.observe(art, { childList: true });
  watch.observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit"] });
  studioDiscoverDress();
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { DISCOVER_KEY_IDS, DISCOVER_STAR_ICONS, DISCOVER_ERROR_ICONS, discoverEmptyLook, discoverKeyNames };
}
