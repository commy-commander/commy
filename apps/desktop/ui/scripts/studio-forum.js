const FORUM_KEYS = { "forum-new": "key sm primary" };
const FORUM_KEY_NAMES = ["key", "sm", "primary"];
const FORUM_ICONS = [["forum-new", "plus", "afterbegin"], ["forum-mode-gallery", "layers", "beforeend"]];

function forumKeyNames(classes, roomy) {
  const wanted = roomy ? String(classes || "").split(" ") : [];
  return Object.fromEntries(FORUM_KEY_NAMES.map(name => [name, wanted.includes(name)]));
}

function studioForumDress() {
  const roomy = document.documentElement.dataset.fit !== "slim";
  for (const [id, classes] of Object.entries(FORUM_KEYS)) {
    const key = document.getElementById(id);
    if (!key) continue;
    for (const [name, on] of Object.entries(forumKeyNames(classes, roomy))) {
      if (key.classList.contains(name) !== on) key.classList.toggle(name, on);
    }
  }
  if (typeof studioIconMarkup !== "function") return;
  for (const [id, icon, where] of FORUM_ICONS) {
    const node = document.getElementById(id);
    if (node && !node.querySelector(":scope > .fv-i")) node.insertAdjacentHTML(where, studioIconMarkup(icon, "fv-i"));
  }
}

function studioForum() {
  studioForumDress();
  new MutationObserver(studioForumDress).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit"] });
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { FORUM_KEYS, FORUM_KEY_NAMES, FORUM_ICONS, forumKeyNames };
}
