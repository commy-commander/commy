const SETTINGS_PAGE_ICONS = {
  "user-settings": {
    account: "user",
    "commy-account": "key-round",
    privacy: "shield",
    voice: "mic",
    camera: "video",
    overlay: "gamepad-2",
    notifications: "bell",
    sounds: "volume-2",
    tts: "speech",
    appearance: "palette",
    accessibility: "type",
    keybinds: "keyboard",
    integrations: "plug",
    storage: "folder",
    updates: "rotate-cw",
    diagnostics: "terminal",
  },
  "server-settings": {
    overview: "info",
    channels: "hash",
    members: "users",
    joins: "user-check",
    roles: "tag",
    invites: "link",
    discovery: "compass",
    onboarding: "door-open",
    rules: "scroll-text",
    emoji: "smile",
    stickers: "image",
    emoticons: "laugh",
    soundboard: "audio-lines",
    removed: "ban",
    automod: "shield-check",
    reports: "flag",
    audit: "file-text",
    bots: "bot",
    "file-storage": "folder",
    "gif-search": "gif",
    "game-database": "gamepad-2",
    "leave-server-btn": "log-out",
  },
  "channel-settings": {
    overview: "info",
    permissions: "lock",
    "channel-delete-nav": "trash-2",
  },
};

const SETTINGS_PICK_ICONS = { "input-device": "mic", "output-device": "headphones" };
const SETTINGS_FILTER_ICONS = { off: "x", higher: "arrow-up", lower: "arrow-down", robot: "bot", radio: "radio", cave: "mountain", whisper: "wind" };
const METER_BARS = 40;

function settingsPageIcon(layerId, tab) {
  const icons = SETTINGS_PAGE_ICONS[layerId] || {};
  return Object.prototype.hasOwnProperty.call(icons, tab) ? icons[tab] : "";
}

function settingsPaneGroups(kinds) {
  const groups = [];
  let open = null;
  kinds.forEach((kind, index) => {
    if (kind === "title") {
      open = null;
      return;
    }
    if (kind === "head" || !open) {
      open = [];
      groups.push(open);
    }
    open.push(index);
  });
  return groups;
}

function meterBarClass(index, lit, count = METER_BARS) {
  if (index >= lit) return "";
  if (index >= count * 0.9) return "on hot";
  if (index >= count * 0.74) return "on warm";
  return "on";
}

function meterLit(percent, count = METER_BARS) {
  const share = Number.isFinite(percent) ? Math.max(0, Math.min(100, percent)) / 100 : 0;
  return Math.round(share * count);
}

function settingsChildKind(node) {
  if (node.matches("h1, .settings-intro")) return "title";
  if (node.matches(".settings-subhead")) return "head";
  return "part";
}

function studioSettingsPanes(page) {
  const kids = [...page.children];
  const groups = settingsPaneGroups(kids.map(settingsChildKind));
  for (const group of groups) {
    const pane = document.createElement("div");
    pane.className = "set-pane slab thin";
    kids[group[0]].before(pane);
    for (const index of group) pane.appendChild(kids[index]);
  }
}

function studioSettingsIcons(layer) {
  for (const tab of layer.querySelectorAll(".settings-nav .settings-tab")) {
    if (tab.querySelector(":scope > .i")) continue;
    const name = settingsPageIcon(layer.id, tab.dataset.tab || tab.id);
    if (!name || typeof studioIconMarkup !== "function") continue;
    tab.insertAdjacentHTML("afterbegin", studioIconMarkup(name));
  }
}

function studioSettingsNav(layer) {
  studioSettingsIcons(layer);
  const nav = layer.querySelector(".settings-nav");
  if (nav) new MutationObserver(() => studioSettingsIcons(layer)).observe(nav, { childList: true, subtree: true });
  const inner = layer.querySelector(".settings-nav-inner");
  if (layer.id === "user-settings" && inner && !inner.querySelector(".settings-nav-title")) {
    const head = document.createElement("div");
    head.className = "settings-nav-title";
    head.dataset.i18n = "nav.settings";
    head.textContent = typeof t === "function" ? t("nav.settings") : "Settings";
    inner.prepend(head);
  }
}

function studioSettingsPick(select) {
  if (select.parentNode && select.parentNode.classList.contains("set-pick")) return;
  if (typeof studioIconMarkup !== "function") return;
  const pick = document.createElement("span");
  pick.className = "set-pick";
  select.before(pick);
  const lead = SETTINGS_PICK_ICONS[select.id];
  if (lead) pick.insertAdjacentHTML("beforeend", studioIconMarkup(lead));
  pick.appendChild(select);
  pick.insertAdjacentHTML("beforeend", studioIconMarkup("chevron-down"));
}

function studioSettingsPicks(layer) {
  for (const select of layer.querySelectorAll(".settings-content select")) studioSettingsPick(select);
  new MutationObserver(records => {
    for (const record of records) {
      for (const node of record.addedNodes) {
        if (node.nodeType !== 1) continue;
        if (node.matches("select")) studioSettingsPick(node);
        else for (const select of node.querySelectorAll("select")) studioSettingsPick(select);
      }
    }
  }).observe(layer.querySelector(".settings-content") || layer, { childList: true, subtree: true });
}

function studioSettingsMeterBars() {
  const meter = document.getElementById("mic-meter");
  if (!meter || meter.querySelector(":scope > i")) return;
  for (let index = 0; index < METER_BARS; index += 1) meter.appendChild(document.createElement("i"));
}

function studioSettingsMeter(percent) {
  const meter = document.getElementById("mic-meter");
  if (!meter) return;
  const lit = meterLit(percent);
  meter.querySelectorAll(":scope > i").forEach((bar, index) => {
    const name = meterBarClass(index, lit);
    if (bar.className !== name) bar.className = name;
  });
}

function studioSettingsFilterKey(button) {
  if (!button.querySelector(":scope > .led")) {
    const name = SETTINGS_FILTER_ICONS[button.dataset.filter];
    if (name && typeof studioIconMarkup === "function") button.insertAdjacentHTML("afterbegin", studioIconMarkup(name));
    const led = document.createElement("i");
    led.className = "led";
    button.appendChild(led);
    new MutationObserver(() => studioSettingsFilterKey(button)).observe(button, { attributes: true, attributeFilter: ["class"] });
  }
  const roomy = document.documentElement.dataset.fit !== "slim";
  const down = roomy && button.classList.contains("active");
  if (button.classList.contains("key") !== roomy) button.classList.toggle("key", roomy);
  if (button.classList.contains("latch") !== roomy) button.classList.toggle("latch", roomy);
  if (button.classList.contains("is-down") !== down) button.classList.toggle("is-down", down);
}

function studioSettingsFilters() {
  const group = document.getElementById("voice-filter");
  if (!group) return;
  const dress = () => { for (const button of group.querySelectorAll(":scope > button")) studioSettingsFilterKey(button); };
  dress();
  new MutationObserver(dress).observe(group, { childList: true });
  new MutationObserver(dress).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit"] });
}

function studioSettingsSide(layer) {
  const app = document.querySelector(".app");
  const paint = () => {
    if (layer.classList.contains("hidden") || !app) return;
    const side = getComputedStyle(app).getPropertyValue("--sidebar-w").trim();
    if (side) layer.style.setProperty("--set-side", side);
  };
  new MutationObserver(paint).observe(layer, { attributes: true, attributeFilter: ["class"] });
  window.addEventListener("resize", paint);
}

function studioSettingsDone(close) {
  const actions = document.querySelector(".user-panel .user-actions");
  if (!actions || actions.querySelector(".set-done")) return;
  const done = document.createElement("button");
  done.type = "button";
  done.className = "key sm cream set-done";
  done.dataset.i18n = "settings.done";
  done.textContent = typeof t === "function" ? t("settings.done") : "Done";
  done.onclick = () => close();
  actions.appendChild(done);
}

function studioSettingsShell(close) {
  for (const layer of document.querySelectorAll(".settings-layer")) {
    studioSettingsNav(layer);
    for (const page of layer.querySelectorAll(".settings-page")) studioSettingsPanes(page);
    studioSettingsPicks(layer);
    studioSettingsSide(layer);
  }
  studioSettingsMeterBars();
  studioSettingsFilters();
  studioSettingsDone(close);
  const rail = document.querySelector(".app > .rail");
  if (rail) {
    rail.addEventListener("click", e => {
      if (e.target.closest(".set-done, #settings-btn, .user-chip")) return;
      if (document.documentElement.dataset.fit === "slim") return;
      if (document.querySelector(".settings-layer:not(.hidden)")) close();
    }, true);
  }
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { SETTINGS_PAGE_ICONS, SETTINGS_PICK_ICONS, SETTINGS_FILTER_ICONS, METER_BARS, settingsPageIcon, settingsPaneGroups, meterBarClass, meterLit };
}
