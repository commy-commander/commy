const SWITCHER_NEWS_FACES = 2;

const phoneSwitcher = { step: false, shown: false, home: null, drag: null };

function newsServers(servers, currentId, unreadOf, mutedOf) {
  return servers
    .filter(server => server.id !== currentId && !mutedOf(server.id))
    .map(server => ({ server, count: unreadOf(server.id) }))
    .filter(item => item.count > 0);
}

function newsSummary(items, faces = SWITCHER_NEWS_FACES) {
  const total = items.reduce((sum, item) => sum + item.count, 0);
  return { faces: items.slice(0, faces).map(item => item.server), total, label: total > 99 ? "99+" : String(total) };
}

function newsCount(summary) {
  return `${summary.label} new`;
}

function newsWords(items) {
  if (!items.length) return "Your servers";
  const names = items.map(item => serverShownName(item.server.name));
  return `New in ${names.length > 2 ? `${names.slice(0, 2).join(", ")} and ${names.length - 2} more` : names.join(" and ")}`;
}

function switcherOn() {
  const app = $("app");
  return Boolean(app) && studioSlim() && app.dataset.pane === "list" && studioPhone.tab === "home" && !app.classList.contains("dm-mode") && Boolean(plainServer());
}

function buildServerSwitcher() {
  const box = el("div", "pt-servers");
  box.id = "pt-servers";
  box.hidden = true;
  const scrim = el("div", "pt-servers-scrim");
  const sheet = el("section", "pt-servers-sheet");
  sheet.id = "pt-servers-sheet";
  sheet.setAttribute("role", "dialog");
  sheet.setAttribute("aria-label", "Your servers");
  const grab = el("div", "pt-servers-grab");
  grab.setAttribute("aria-hidden", "true");
  const head = el("div", "pt-servers-head");
  const title = el("h2", "pt-servers-title", "Your servers");
  const options = studioNode("button", "key sq pt-servers-options", '<i data-i="ellipsis"></i>');
  options.type = "button";
  options.id = "pt-servers-options";
  options.setAttribute("aria-label", "Server options");
  options.onclick = () => closeServerSwitcher(() => openServerMenu(state.currentServer, 0, 0));
  head.append(title, options);
  const slot = el("div", "pt-servers-slot");
  slot.id = "pt-servers-slot";
  const keys = el("div", "pt-servers-keys");
  keys.append(switcherKey("plus", "Add a server", "add-server-btn"), switcherKey("compass", "Discover", "discover-btn"));
  sheet.append(grab, head, slot, keys);
  box.append(scrim, sheet);
  scrim.onclick = () => closeServerSwitcher();
  slot.addEventListener("click", e => {
    if (e.target.closest(".server-icon:not(.waiting)")) closeServerSwitcher();
  });
  slot.addEventListener("contextmenu", e => {
    const tile = e.target.closest(".server-icon:not(.waiting), .folder-head");
    if (!tile) return;
    e.preventDefault();
    e.stopPropagation();
    closeServerSwitcher(() => tile.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: e.clientX, clientY: e.clientY, button: 2 })));
  }, true);
  wireSwitcherGrab(grab, sheet);
  $("app").appendChild(box);

  const news = el("button", "pt-news");
  news.id = "pt-news";
  news.type = "button";
  news.hidden = true;
  news.onclick = () => openServerSwitcher();
  $("server-name-header").appendChild(news);
  $("server-name-header").addEventListener("contextmenu", e => {
    if (!switcherOn() || e.target.closest("button")) return;
    e.preventDefault();
    openServerMenu(state.currentServer, e.clientX, e.clientY);
  });
}

function optionsWords(server) {
  return server ? `${serverShownName(server.name)} options` : "Server options";
}

function switcherKey(icon, words, target) {
  const key = studioNode("button", "pt-servers-key", `<i data-i="${icon}"></i>`);
  key.type = "button";
  key.appendChild(el("span", "", words));
  key.onclick = () => closeServerSwitcher(() => $(target).click());
  return key;
}

function paintNewsCapsule() {
  const news = $("pt-news");
  const current = plainServer();
  const items = current ? newsServers(railServers(), current.id, serverUnreadCount, isServerMuted) : [];
  news.hidden = !switcherOn() || !items.length;
  news.setAttribute("aria-label", newsWords(items));
  news.title = newsWords(items);
  if (news.hidden) return;
  const summary = newsSummary(items);
  const faces = summary.faces.map(server => {
    const face = el("span", "pt-news-face");
    paintServerIcon(face, server);
    face.style.setProperty("--server-tint", serverTint(server.id));
    return face;
  });
  news.replaceChildren(...faces, el("span", "pt-count pt-news-count", newsCount(summary)));
}

function paintSwitcher() {
  const on = switcherOn();
  $("app").classList.toggle("pt-switch", on);
  if (!on && phoneSwitcher.shown) closeServerSwitcher();
  paintNewsCapsule();
}

function openServerSwitcher() {
  if (!switcherOn() || phoneSwitcher.shown) return;
  const list = $("server-list");
  phoneSwitcher.home = { parent: list.parentNode, next: list.nextSibling };
  $("pt-servers-slot").appendChild(list);
  phoneSwitcher.shown = true;
  $("pt-servers-options").setAttribute("aria-label", optionsWords(plainServer()));
  $("pt-servers").hidden = false;
  $("pt-servers-sheet").style.translate = "";
  if (layerStep("ptServers", true, phoneSwitcher.step, history.state) === "push") history.pushState({ ...(history.state || { pane: phonePane() }), ptServers: true }, "");
  phoneSwitcher.step = true;
}

function stowServerSwitcher() {
  if (!phoneSwitcher.shown) return;
  phoneSwitcher.shown = false;
  $("pt-servers").hidden = true;
  const home = phoneSwitcher.home;
  phoneSwitcher.home = null;
  if (home) home.parent.insertBefore($("server-list"), home.next && home.next.parentNode === home.parent ? home.next : null);
}

function closeServerSwitcher(then) {
  const held = phoneSwitcher.step;
  stowServerSwitcher();
  phoneSwitcher.step = false;
  if (layerStep("ptServers", false, held, history.state) === "back") {
    if (then) window.addEventListener("popstate", () => then(), { once: true });
    history.back();
    return;
  }
  if (then) then();
}

function wireSwitcherGrab(grab, sheet) {
  grab.addEventListener("pointerdown", e => {
    if (!phoneSwitcher.shown) return;
    phoneSwitcher.drag = { id: e.pointerId, y: e.clientY, dy: 0, samples: [{ t: e.timeStamp, x: e.clientX, y: e.clientY }] };
    grab.setPointerCapture(e.pointerId);
    sheet.classList.add("pt-dragging");
  });
  grab.addEventListener("pointermove", e => {
    const drag = phoneSwitcher.drag;
    if (!drag || drag.id !== e.pointerId) return;
    drag.dy = e.clientY - drag.y;
    drag.samples = keepSamples(drag.samples, { t: e.timeStamp, x: e.clientX, y: e.clientY });
    sheet.style.translate = `0 ${sheetPull(drag.dy)}px`;
  });
  const end = cancelled => e => {
    const drag = phoneSwitcher.drag;
    if (!drag || drag.id !== e.pointerId) return;
    phoneSwitcher.drag = null;
    sheet.classList.remove("pt-dragging");
    if (!cancelled && sheetSettles(drag.dy, dragSpeed(drag.samples, e.timeStamp).y, SWIPE.sheetAt)) closeServerSwitcher();
    else sheet.style.translate = "";
  };
  grab.addEventListener("pointerup", end(false));
  grab.addEventListener("pointercancel", end(true));
}

function wireServerSwitcher() {
  if (!$("app") || !$("server-list") || !$("server-name-header")) return;
  buildServerSwitcher();
  const later = () => requestAnimationFrame(paintSwitcher);
  new MutationObserver(later).observe($("app"), { attributes: true, attributeFilter: ["class", "data-tab", "data-pane"] });
  new MutationObserver(later).observe($("server-list"), { childList: true, subtree: true });
  new MutationObserver(later).observe($("server-name"), { childList: true, characterData: true, subtree: true });
  new MutationObserver(later).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit", "data-look"] });
  window.addEventListener("popstate", () => {
    if ((history.state && history.state.ptServers) || !phoneSwitcher.shown) return;
    phoneSwitcher.step = false;
    stowServerSwitcher();
  });
  paintSwitcher();
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { SWITCHER_NEWS_FACES, newsServers, newsSummary, newsCount, newsWords, optionsWords };
} else {
  wireServerSwitcher();
}
