const START_KEYS = { "btn-primary": "key primary", "btn-secondary": "key" };

function startStarState(cardId) {
  return cardId === "reopen" ? "waiting" : "working";
}

function startKeyFor(classList) {
  const names = Array.from(classList || []);
  for (const [name, classes] of Object.entries(START_KEYS)) if (names.includes(name)) return classes;
  return null;
}

function startGhostsWanted({ query = "", rows = 0, empty = false } = {}) {
  return Boolean(empty) && rows === 0 && String(query).trim() === "";
}

function startStar(host, state, before) {
  let star = host.querySelector(":scope > .start-star");
  if (star) return star;
  star = document.createElement("span");
  star.className = "star start-star";
  star.setAttribute("aria-hidden", "true");
  star.dataset.state = state;
  star.dataset.follow = "off";
  if (before && before.parentElement === host) before.before(star);
  else host.prepend(star);
  if (typeof Star !== "undefined") Star.mount(star);
  return star;
}

function startRoomy() {
  return document.documentElement.dataset.fit !== "slim";
}

function studioStartSplash() {
  for (const cover of document.querySelectorAll(".startup")) {
    if (!cover.querySelector(":scope > .start-room") && typeof studioRoomMarkup === "function") {
      const room = document.createElement("div");
      room.className = "start-room";
      room.setAttribute("aria-hidden", "true");
      room.innerHTML = studioRoomMarkup();
      cover.prepend(room);
    }
    const card = cover.querySelector(":scope > .startup-card");
    if (card) startStar(card, startStarState(cover.id), card.querySelector(":scope > .startup-logo"));
  }
}

function studioStartKeys(root) {
  const roomy = startRoomy();
  const skip = document.getElementById("startup-skip");
  if (skip) studioKeyDress(skip, "key", roomy);
  if (!root) return;
  for (const key of root.querySelectorAll(":scope > :is(.welcome-home, .dm-empty) :is(.btn-primary, .btn-secondary)")) {
    const classes = startKeyFor(key.classList);
    if (classes) studioKeyDress(key, classes, roomy);
  }
}

function studioStartFeed() {
  const feed = document.getElementById("messages");
  if (!feed) return;
  for (const intro of feed.querySelectorAll(":scope > .channel-intro")) startStar(intro, "waiting");
  for (const home of feed.querySelectorAll(":scope > .welcome-home")) startStar(home, "waiting", home.querySelector(":scope > .welcome-mark"));
  studioStartKeys(feed);
}

function studioStartList() {
  const list = document.getElementById("channel-list");
  const search = document.getElementById("dm-search");
  if (!list) return;
  const empty = list.querySelector(":scope > .dm-list-empty");
  const rows = list.querySelectorAll(":scope > .dm-thread:not(.saved-entry), :scope > .friend-request").length;
  const wanted = startGhostsWanted({ query: search ? search.value : "", rows, empty: Boolean(empty) });
  const ghosts = list.querySelector(":scope > .start-ghosts");
  if (!wanted) {
    if (ghosts) ghosts.remove();
    return;
  }
  if (ghosts && ghosts.nextElementSibling === empty) return;
  if (ghosts) ghosts.remove();
  const pile = document.createElement("div");
  pile.className = "start-ghosts";
  pile.setAttribute("aria-hidden", "true");
  for (let i = 0; i < 7; i++) pile.insertAdjacentHTML("beforeend", "<div class=\"start-ghost\"><i></i><span><b></b><small></small></span></div>");
  empty.before(pile);
}

function studioStartFind() {
  const search = document.getElementById("dm-search");
  if (!search || !search.parentElement || search.parentElement.querySelector(":scope > .start-find")) return;
  const icon = document.createElement("span");
  icon.className = "start-find";
  icon.setAttribute("aria-hidden", "true");
  if (typeof studioIconMarkup === "function") icon.innerHTML = studioIconMarkup("search");
  search.after(icon);
}

function studioStart() {
  studioStartSplash();
  studioStartFind();
  studioStartFeed();
  studioStartList();
  const feed = document.getElementById("messages");
  if (feed) new MutationObserver(studioStartFeed).observe(feed, { childList: true });
  const list = document.getElementById("channel-list");
  if (list) new MutationObserver(studioStartList).observe(list, { childList: true });
  new MutationObserver(() => studioStartKeys(document.getElementById("messages"))).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit"] });
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { START_KEYS, startStarState, startKeyFor, startGhostsWanted };
}
