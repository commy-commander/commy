function dressPhoneForum() {
  if (typeof document === "undefined") return;
  const add = document.getElementById("forum-new");
  const gallery = document.getElementById("forum-mode-gallery");
  if (!add || !gallery || typeof studioIconMarkup !== "function") return;
  const slim = typeof studioSlim === "function" && studioSlim();
  const plus = add.querySelector(":scope > .pf-plus");
  if (slim && !plus) add.insertAdjacentHTML("afterbegin", studioIconMarkup("plus", "pf-plus"));
  if (!slim && plus) plus.remove();
  const layers = gallery.querySelector(":scope > .pf-layers");
  if (slim && !layers) gallery.innerHTML = studioIconMarkup("layers", "pf-layers");
  if (!slim && layers && typeof setIcon === "function") setIcon(gallery, "grid", 18);
}

if (typeof document !== "undefined" && typeof MutationObserver !== "undefined") {
  const watch = new MutationObserver(dressPhoneForum);
  watch.observe(document.documentElement, { attributes: true, attributeFilter: ["data-look", "data-fit"] });
  for (const id of ["forum-new", "forum-mode-gallery"]) {
    const node = document.getElementById(id);
    if (node) watch.observe(node, { childList: true });
  }
  dressPhoneForum();
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { dressPhoneForum };
}
