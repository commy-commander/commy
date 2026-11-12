const PHONE_SETTINGS_LAYERS = ["user-settings", "server-settings", "channel-settings"];
const PHONE_SETTINGS_FROM_MENUS = { "sm-settings": "server-settings", "chm-settings": "channel-settings" };
let phoneSettingsWantsMenu = "";

function phoneSettingsMore(scroller) {
  if (!scroller) return;
  scroller.classList.toggle("pt-more", scroller.scrollTop + scroller.clientHeight < scroller.scrollHeight - 2);
}

function phoneSettingsChannelTitle(title) {
  const nodes = [...$("channel-settings-title").childNodes].map(node => node.cloneNode(true));
  const first = nodes[0];
  if (first && first.nodeType === 3 && first.textContent.startsWith("#")) {
    nodes.splice(0, 1, el("em", "", "#"));
  }
  title.replaceChildren(...nodes);
}

function phoneSettingsSub(layer, page) {
  if (layer.id === "server-settings") return page ? $("server-settings-title").textContent : layer.getAttribute("aria-label");
  if (layer.id === "channel-settings" && !page) return layer.getAttribute("aria-label");
  return "";
}

function phoneSettingsChevrons(layer, slim) {
  for (const tab of layer.querySelectorAll(".settings-nav .settings-tab")) {
    const chev = tab.querySelector(":scope > .pt-set-chev");
    const wants = slim && !tab.classList.contains("danger");
    if (wants && !chev) {
      const holder = el("span", "pt-set-chev");
      holder.innerHTML = studioIconMarkup("chevron-right");
      tab.append(holder);
    } else if (!wants && chev) {
      chev.remove();
    }
  }
}

function phoneSettingsHead(layer) {
  const bar = layer.querySelector(".settings-phone-bar");
  const slim = studioSlim();
  phoneSettingsChevrons(layer, slim);
  if (!bar) return;
  const title = bar.querySelector(".settings-phone-title");
  let sub = bar.querySelector(".settings-phone-sub");
  if (!slim) {
    if (sub) sub.remove();
    return;
  }
  if (!sub) {
    sub = el("span", "settings-phone-sub");
    title.after(sub);
  }
  const page = layer.dataset.phoneView === "page";
  if (layer.id === "channel-settings" && !page) phoneSettingsChannelTitle(title);
  sub.textContent = phoneSettingsSub(layer, page);
  phoneSettingsMore(layer.querySelector(page ? ".settings-content-inner" : ".settings-nav-inner"));
}

function phoneSettingsTurn(layer) {
  if (!studioSlim()) return;
  const inner = layer.querySelector(".settings-content-inner");
  if (inner) inner.scrollTop = 0;
}

function wirePhoneSettings() {
  for (const id of PHONE_SETTINGS_LAYERS) {
    const layer = $(id);
    if (!layer) continue;
    let tab = layer.dataset.tab;
    new MutationObserver(() => {
      if (phoneSettingsWantsMenu === id && !layer.classList.contains("hidden")) {
        phoneSettingsWantsMenu = "";
        if (studioSlim()) settingsPhoneView(layer, "menu");
      }
      if (layer.dataset.tab !== tab) {
        tab = layer.dataset.tab;
        phoneSettingsTurn(layer);
      }
      phoneSettingsHead(layer);
    }).observe(layer, { attributes: true, attributeFilter: ["data-phone-view", "data-tab", "class"] });
    layer.addEventListener("click", e => {
      const back = e.target.closest(".settings-phone-back");
      if (!back || !studioSlim() || layer.dataset.phoneView === "page") return;
      e.preventDefault();
      e.stopPropagation();
      layer.querySelector(".settings-phone-close").click();
    }, true);
    for (const scroller of layer.querySelectorAll(".settings-nav-inner, .settings-content-inner")) {
      scroller.addEventListener("scroll", () => phoneSettingsMore(scroller), { passive: true });
    }
  }
  for (const [key, id] of Object.entries(PHONE_SETTINGS_FROM_MENUS)) {
    const row = $(key);
    if (row) row.addEventListener("click", () => { phoneSettingsWantsMenu = studioSlim() ? id : ""; }, true);
  }
  new MutationObserver(() => {
    for (const id of PHONE_SETTINGS_LAYERS) if ($(id)) phoneSettingsHead($(id));
  }).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit", "data-look"] });
}

wirePhoneSettings();
