const STUDIO_COMPACT_MARK = "data-studio-compact";
let studioCompactStarted = false;
const STUDIO_COMPACT_ROWS = [
  { key: "threads", button: "threads-btn", label: "Threads", icon: "thread", open: "toggleThreadsPanel" },
  { key: "pins", button: "pins-btn", label: "Pinned Messages", icon: "pin", open: "togglePinsPanel" },
  { key: "saved", button: "saved-btn", label: "Saved Messages", icon: "bookmark", open: "toggleSavedView" },
];
const STUDIO_COMPACT_MIXED_LABEL = "Pins and Saved";

function studioCompactRowStates(rows, read) {
  return rows.map(row => {
    const seen = read(row) || {};
    const shown = Boolean(seen.shown);
    return { key: row.key, shown, badge: shown && seen.badge ? String(seen.badge).trim() : "" };
  });
}

function studioCompactDot(states) {
  return states.some(state => state.shown && state.badge !== "");
}

function studioCompactEmpty(states) {
  return !states.some(state => state.shown);
}

function studioCompactFace(rows, states) {
  const shown = rows.filter(row => states.some(state => state.key === row.key && state.shown));
  if (!shown.length) return null;
  const lead = shown[0];
  if (shown.length === 1) return { icon: lead.icon, label: lead.label, single: lead.key };
  return { icon: lead.icon, label: lead.key === "threads" ? lead.label : STUDIO_COMPACT_MIXED_LABEL, single: null };
}

function studioCompactMenuLeft(anchor, width, viewport) {
  return Math.max(8, Math.min(anchor.right - width, viewport - width - 8));
}

function studioCompactIcon(name, size) {
  if (typeof studioIconNode === "function") {
    const icon = studioIconNode(name, size, false);
    if (icon) return icon;
  }
  return typeof svgIcon === "function" ? svgIcon(name, size) : null;
}

function studioCompactReadRow(row) {
  const button = document.getElementById(row.button);
  if (!button) return { shown: false, badge: "" };
  const badge = button.querySelector(".pin-count:not(.hidden)");
  return { shown: !button.classList.contains("hidden"), badge: badge ? badge.textContent : "" };
}

function studioCompactSync() {
  const more = document.getElementById("studio-more-btn");
  const menu = document.getElementById("studio-more-menu");
  if (!more || !menu) return;
  const states = studioCompactRowStates(STUDIO_COMPACT_ROWS, studioCompactReadRow);
  for (const state of states) {
    const row = menu.querySelector(`[data-row="${state.key}"]`);
    if (!row) continue;
    row.classList.toggle("hidden", !state.shown);
    const badge = row.querySelector(".studio-more-badge");
    badge.textContent = state.badge;
    badge.classList.toggle("hidden", state.badge === "");
  }
  studioCompactPaintFace(more, studioCompactFace(STUDIO_COMPACT_ROWS, states));
  more.toggleAttribute("data-empty", studioCompactEmpty(states));
  more.toggleAttribute("data-dot", studioCompactDot(states));
  const lit = !menu.classList.contains("hidden") || STUDIO_COMPACT_ROWS.some(row => {
    const button = document.getElementById(row.button);
    return Boolean(button && button.classList.contains("active"));
  });
  more.classList.toggle("active", lit);
  more.setAttribute("aria-expanded", String(!menu.classList.contains("hidden")));
  if (studioCompactEmpty(states)) menu.classList.add("hidden");
}

function studioCompactPaintFace(more, face) {
  const key = face ? `${face.icon}|${face.label}|${face.single || ""}` : "";
  if (more.dataset.face === key) return;
  more.dataset.face = key;
  more.replaceChildren();
  more.toggleAttribute("data-single", Boolean(face && face.single));
  if (!face) return;
  more.title = face.label;
  more.setAttribute("aria-label", face.label);
  more.setAttribute("aria-haspopup", face.single ? "dialog" : "menu");
  const icon = studioCompactIcon(face.icon, 20);
  if (icon) more.appendChild(icon);
  if (face.single) return;
  const caret = studioCompactIcon("chevronDown", 12);
  if (!caret) return;
  caret.classList.add("studio-more-caret");
  more.appendChild(caret);
}

function studioCompactCloseMore() {
  const menu = document.getElementById("studio-more-menu");
  if (!menu || menu.classList.contains("hidden")) return;
  menu.classList.add("hidden");
  studioCompactSync();
}

function studioCompactToggleMore() {
  const more = document.getElementById("studio-more-btn");
  const menu = document.getElementById("studio-more-menu");
  if (!more || !menu) return;
  if (!menu.classList.contains("hidden")) return studioCompactCloseMore();
  const states = studioCompactRowStates(STUDIO_COMPACT_ROWS, studioCompactReadRow);
  const face = studioCompactFace(STUDIO_COMPACT_ROWS, states);
  if (face && face.single) return studioCompactPickRow(STUDIO_COMPACT_ROWS.find(row => row.key === face.single));
  if (typeof closeFloating === "function") closeFloating();
  studioCompactSync();
  menu.classList.remove("hidden");
  const anchor = more.getBoundingClientRect();
  const width = menu.getBoundingClientRect().width;
  menu.style.top = `${Math.round(anchor.bottom + 8)}px`;
  menu.style.left = `${Math.round(studioCompactMenuLeft(anchor, width, window.innerWidth))}px`;
  studioCompactSync();
}

function studioCompactPickRow(row) {
  studioCompactCloseMore();
  const open = window[row.open];
  if (typeof open === "function") open();
  studioCompactSync();
}

function studioCompactOpenTextStyle() {
  if (typeof closeFloating === "function") closeFloating();
  const button = document.getElementById("text-style-btn");
  const pop = document.getElementById("text-style-pop");
  if (!button || !pop) return;
  button.click();
  const field = document.getElementById("message-input");
  if (pop.classList.contains("hidden") || !field || typeof placePopup !== "function") return;
  placePopup(pop, field, "above");
  pop.style.left = `${Math.round(studioCompactMenuLeft(field.getBoundingClientRect(), pop.getBoundingClientRect().width, window.innerWidth))}px`;
}

function studioCompactBuildMenu() {
  const menu = document.createElement("div");
  menu.id = "studio-more-menu";
  menu.className = "pop-menu ctx-menu studio-more-menu hidden";
  menu.setAttribute("role", "menu");
  menu.setAttribute(STUDIO_COMPACT_MARK, "");
  for (const row of STUDIO_COMPACT_ROWS) {
    const item = document.createElement("button");
    item.type = "button";
    item.dataset.row = row.key;
    item.setAttribute("role", "menuitem");
    const icon = studioCompactIcon(row.icon, 18);
    if (icon) item.appendChild(icon);
    const label = document.createElement("span");
    label.className = "studio-more-label";
    label.textContent = row.label;
    item.appendChild(label);
    const badge = document.createElement("span");
    badge.className = "studio-more-badge hidden";
    item.appendChild(badge);
    item.addEventListener("click", () => studioCompactPickRow(row));
    menu.appendChild(item);
  }
  return menu;
}

function studioCompactAdd() {
  const actions = document.querySelector(".chat-header .header-actions");
  if (actions && !document.getElementById("studio-more-btn")) {
    const more = document.createElement("button");
    more.type = "button";
    more.id = "studio-more-btn";
    more.className = "icon-btn studio-more-btn";
    more.setAttribute("aria-expanded", "false");
    more.setAttribute(STUDIO_COMPACT_MARK, "");
    more.addEventListener("click", studioCompactToggleMore);
    const saved = document.getElementById("saved-btn");
    actions.insertBefore(more, saved && saved.parentNode === actions ? saved.nextSibling : null);
  }
  if (!document.getElementById("studio-more-menu")) document.body.appendChild(studioCompactBuildMenu());
  const plus = document.getElementById("composer-menu");
  if (plus && !document.getElementById("studio-cm-style")) {
    const item = document.createElement("button");
    item.type = "button";
    item.id = "studio-cm-style";
    item.setAttribute("role", "menuitem");
    item.setAttribute(STUDIO_COMPACT_MARK, "");
    const icon = studioCompactIcon("a-large-small", 18);
    if (icon) item.appendChild(icon);
    item.appendChild(document.createTextNode("Text Style"));
    item.addEventListener("click", studioCompactOpenTextStyle);
    plus.appendChild(item);
  }
  studioCompactSync();
}

function studioCompactUndo() {
  for (const node of document.querySelectorAll(`[${STUDIO_COMPACT_MARK}]`)) node.remove();
}

function studioCompactFit() {
  if (document.documentElement.dataset.fit === "slim") studioCompactUndo();
  else studioCompactAdd();
}

function studioCompact() {
  if (studioCompactStarted) return;
  studioCompactStarted = true;
  studioCompactFit();
  new MutationObserver(studioCompactFit).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit"] });
  const watch = new MutationObserver(studioCompactSync);
  for (const row of STUDIO_COMPACT_ROWS) {
    const button = document.getElementById(row.button);
    if (button) watch.observe(button, { attributes: true, attributeFilter: ["class"], childList: true, subtree: true, characterData: true });
  }
  document.addEventListener("mousedown", event => {
    const menu = document.getElementById("studio-more-menu");
    if (!menu || menu.classList.contains("hidden")) return;
    if (menu.contains(event.target) || event.target.closest("#studio-more-btn")) return;
    studioCompactCloseMore();
  });
  document.addEventListener("keydown", event => {
    if (event.key === "Escape") studioCompactCloseMore();
  });
  window.addEventListener("resize", studioCompactCloseMore);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { STUDIO_COMPACT_MARK, STUDIO_COMPACT_ROWS, STUDIO_COMPACT_MIXED_LABEL, studioCompactFace, studioCompactRowStates, studioCompactDot, studioCompactEmpty, studioCompactMenuLeft };
}
