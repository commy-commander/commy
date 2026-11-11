let phoneDiscoverStar = null;

function phoneDiscoverStarTitles() {
  return { quiet: discoverEmptyView({ unavailable: true }).title, error: discoverEmptyView({ error: "-" }).title };
}

function phoneDiscoverArt() {
  const empty = $("discover-empty");
  const art = $("discover-empty-art");
  if (!empty || !art) return;
  const titles = phoneDiscoverStarTitles();
  const title = $("discover-empty-title").textContent;
  const star = studioSlim() && (title === titles.quiet || title === titles.error);
  if (star) {
    if (!phoneDiscoverStar) {
      phoneDiscoverStar = el("span", "star");
      phoneDiscoverStar.dataset.state = "quiet";
      phoneDiscoverStar.dataset.follow = "off";
      phoneDiscoverStar.setAttribute("aria-hidden", "true");
      if (typeof Star !== "undefined") Star.mount(phoneDiscoverStar);
    }
    if (phoneDiscoverStar.parentElement !== art) art.appendChild(phoneDiscoverStar);
    empty.dataset.ptArt = "star";
  } else {
    if (phoneDiscoverStar && phoneDiscoverStar.parentElement) phoneDiscoverStar.remove();
    delete empty.dataset.ptArt;
  }
  if (star && title === titles.error) empty.dataset.ptTone = "error";
  else delete empty.dataset.ptTone;
}

function wirePhoneDiscover() {
  const empty = $("discover-empty");
  if (!empty || typeof MutationObserver === "undefined") return;
  new MutationObserver(phoneDiscoverArt).observe(empty, { childList: true, subtree: true, characterData: true });
  new MutationObserver(phoneDiscoverArt).observe(document.documentElement, { attributes: true, attributeFilter: ["data-look", "data-fit"] });
  phoneDiscoverArt();
}

wirePhoneDiscover();
