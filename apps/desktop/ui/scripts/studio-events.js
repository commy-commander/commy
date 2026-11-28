const EVENT_HEAD_KEYS = { "event-create": "key sm", "events-close": "key sq" };

const EVENT_HEAD_ICONS = { "event-create": "plus" };

function studioEventIcon(button, name, on) {
  const have = button.querySelector(":scope > svg.studio-event-i");
  if (!on) {
    if (have) have.remove();
    return;
  }
  if (have && have.getAttribute("data-icon") === name) return;
  if (have) have.remove();
  const icon = studioIconNode(name, 16);
  if (!icon) return;
  icon.classList.add("studio-event-i");
  button.prepend(icon);
}

function studioEventsBody(body, roomy) {
  if (!roomy) {
    if (!body.querySelector("[data-studio-event]")) return;
    const layer = document.getElementById("events-layer");
    if (layer && !layer.classList.contains("hidden") && typeof renderEvents === "function") renderEvents();
    else body.replaceChildren();
    return;
  }
  for (const turn of body.querySelectorAll(".cal-head > .icon-btn")) {
    turn.dataset.studioEvent = "on";
    studioKeyDress(turn, "key sq", roomy);
  }
}

function studioEventHead(key, classes, icon, roomy) {
  if (roomy) {
    key.dataset.studioEvent = "on";
    studioKeyDress(key, classes, true);
    if (icon) studioEventIcon(key, icon, true);
    return;
  }
  if (!key.dataset.studioEvent) return;
  delete key.dataset.studioEvent;
  const phone = key.classList.contains("pe-key");
  studioKeyDress(key, classes, false);
  if (phone) key.classList.add("key");
  if (icon) studioEventIcon(key, icon, false);
}

function studioEvents() {
  const layer = document.getElementById("events-layer");
  const body = document.getElementById("events-body");
  if (!layer || !body) return;
  const panel = layer.querySelector(".events-panel");
  const roomyNow = () => document.documentElement.dataset.fit !== "slim";
  const head = () => {
    const roomy = roomyNow();
    for (const [id, classes] of Object.entries(EVENT_HEAD_KEYS)) {
      const key = document.getElementById(id);
      if (key) studioEventHead(key, classes, EVENT_HEAD_ICONS[id], roomy);
    }
  };
  const dress = () => {
    const roomy = roomyNow();
    if (panel) studioToggle(panel, "slab", roomy);
    head();
    studioEventsBody(body, roomy);
  };
  new MutationObserver(() => studioEventsBody(body, roomyNow())).observe(body, { childList: true, subtree: true });
  new MutationObserver(dress).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit"] });
  const create = document.getElementById("event-create");
  if (create) new MutationObserver(() => { if (roomyNow()) head(); }).observe(create, { attributes: true, attributeFilter: ["class"] });
  dress();
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { EVENT_HEAD_KEYS, EVENT_HEAD_ICONS };
}
