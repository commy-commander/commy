const EVENT_HEAD_KEYS = { "event-create": "key sm", "events-close": "key sq" };

const EVENT_HEAD_ICONS = { "event-create": "plus" };

function eventActionKey(index, iconButton, roomy) {
  if (!roomy || iconButton) return "";
  return index < 2 ? "key sm" : "key sm primary";
}

function eventAnswerMine(index, classes, roomy) {
  return Boolean(roomy && index < 2 && String(classes || "").split(" ").includes("btn-primary"));
}

function eventDeleteIcon(roomy) {
  return roomy ? "trash-2" : "trash";
}

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

function studioEventDelete(button, roomy) {
  const want = eventDeleteIcon(roomy);
  const svg = button.querySelector(":scope > svg");
  if (svg && svg.getAttribute("data-icon") === want) return;
  const icon = studioIconNode(want, 16);
  if (!icon) return;
  if (svg) svg.replaceWith(icon);
  else button.appendChild(icon);
}

function studioEventActions(actions, roomy) {
  [...actions.children].forEach((button, index) => {
    const iconButton = button.classList.contains("icon-btn");
    studioKeyDress(button, eventActionKey(index, iconButton, roomy), roomy);
    if (iconButton) {
      if (button.classList.contains("danger")) studioEventDelete(button, roomy);
      return;
    }
    const mine = eventAnswerMine(index, button.className, roomy);
    studioToggle(button, "mine", mine);
    if (roomy && index < 2) button.setAttribute("aria-pressed", String(mine));
    else button.removeAttribute("aria-pressed");
    studioEventIcon(button, "check", mine);
  });
}

function studioEventsBody(body, roomy) {
  for (const actions of body.querySelectorAll(".event-actions")) studioEventActions(actions, roomy);
  for (const turn of body.querySelectorAll(".cal-head > .icon-btn")) studioKeyDress(turn, "key sq", roomy);
}

function studioEvents() {
  const layer = document.getElementById("events-layer");
  const body = document.getElementById("events-body");
  if (!layer || !body) return;
  const panel = layer.querySelector(".events-panel");
  const dress = () => {
    const roomy = document.documentElement.dataset.fit !== "slim";
    if (panel) studioToggle(panel, "slab", roomy);
    for (const [id, classes] of Object.entries(EVENT_HEAD_KEYS)) {
      const key = document.getElementById(id);
      if (!key) continue;
      studioKeyDress(key, classes, roomy);
      if (EVENT_HEAD_ICONS[id]) studioEventIcon(key, EVENT_HEAD_ICONS[id], roomy);
    }
    studioEventsBody(body, roomy);
  };
  new MutationObserver(() => studioEventsBody(body, document.documentElement.dataset.fit !== "slim")).observe(body, { childList: true, subtree: true });
  new MutationObserver(dress).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit"] });
  dress();
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { EVENT_HEAD_KEYS, EVENT_HEAD_ICONS, eventActionKey, eventAnswerMine, eventDeleteIcon };
}
