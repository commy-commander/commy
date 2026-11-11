const PHONE_EVENT_DOTS = 3;
const PHONE_EVENT_FIELDS = { "event-start": "calendar-days", "event-end": "clock" };
const phoneEventSteps = { events: false, editor: false };

function phoneEventWhen(value, locale) {
  if (!value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleString(locale || [], { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
}

function phoneEventDots(titles, more) {
  return Math.min(PHONE_EVENT_DOTS, titles + (more ? 1 : 0));
}

function phoneEventSpareWeeks(others) {
  const spare = [];
  for (let start = others.length - 7; start >= 0; start -= 7) {
    const week = others.slice(start, start + 7);
    if (week.length < 7 || !week.every(Boolean)) break;
    for (let i = start; i < start + 7; i++) spare.push(i);
  }
  return spare.sort((a, b) => a - b);
}

function phoneEventBackCloses(entry, shown) {
  const step = entry || {};
  const editor = Boolean(shown.editor && shown.editorHeld && !step.ptEventEditor);
  const events = Boolean(shown.events && shown.eventsHeld && !step.ptEvents);
  return { editor, events };
}

function phoneEventStepsBack(editorStep, eventsStep) {
  return (editorStep === "back" ? 1 : 0) + (eventsStep === "back" ? 1 : 0);
}

function phoneEventIcon(name, extra) {
  return typeof studioIconMarkup === "function" ? studioIconMarkup(name, `pe-x ${extra || ""}`.trim()) : "";
}

function phoneEventSwapIcon(holder, name, extra) {
  const old = holder.querySelector(":scope > svg");
  if (old) old.remove();
  holder.insertAdjacentHTML("afterbegin", phoneEventIcon(name, extra));
}

function phoneEventsHead() {
  const layer = document.getElementById("events-layer");
  const head = layer && layer.querySelector(".events-head");
  if (!head) return;
  if (!head.querySelector(":scope > .pe-back")) {
    const back = document.createElement("button");
    back.type = "button";
    back.className = "pe-x pe-back";
    back.setAttribute("aria-label", "Back");
    back.innerHTML = phoneEventIcon("chevron-left");
    back.addEventListener("click", () => closeEvents());
    const title = document.createElement("div");
    title.className = "pe-x pe-title";
    title.append(document.createElement("b"), document.createElement("small"));
    title.firstChild.textContent = "Events";
    head.prepend(back, title);
    const slab = document.createElement("span");
    slab.className = "pe-x pe-slab";
    slab.setAttribute("aria-hidden", "true");
    head.after(slab);
  }
  const server = typeof state !== "undefined" && state.currentServer ? state.currentServer.name : "";
  head.querySelector(".pe-title small").textContent = server;
  const create = document.getElementById("event-create");
  if (create && !create.classList.contains("pe-key")) {
    create.classList.remove("btn-primary", "btn-small");
    create.classList.add("key", "pe-key");
    create.insertAdjacentHTML("afterbegin", phoneEventIcon("plus"));
  }
}

function phoneEventsCard(card) {
  if (card.dataset.pe) return;
  card.dataset.pe = "1";
  const foot = card.querySelector(".event-foot");
  if (foot) card.appendChild(foot);
  const where = card.querySelector(".event-where");
  if (where) phoneEventSwapIcon(where, "volume-2");
  const buttons = foot ? [...foot.querySelectorAll(".event-actions > button")] : [];
  const icons = buttons.filter(button => button.classList.contains("icon-btn"));
  buttons.filter(button => !icons.includes(button)).forEach((button, index) => {
    const answer = index < 2;
    const mine = answer && button.classList.contains("btn-primary");
    button.classList.remove("btn-primary", "btn-secondary", "btn-small");
    button.classList.add("key", "sm", answer ? "pe-answer" : "primary");
    if (mine) {
      button.classList.add("mine");
      button.setAttribute("aria-pressed", "true");
      button.insertAdjacentHTML("afterbegin", phoneEventIcon("check"));
    } else if (answer) button.setAttribute("aria-pressed", "false");
  });
  icons.forEach((button, index) => {
    button.classList.remove("danger");
    button.classList.add("pe-ibtn");
    if (index === 0) button.classList.add("pe-push");
    phoneEventSwapIcon(button, index === 0 ? "pencil" : "trash-2");
  });
}

function phoneEventsCalendar(cal) {
  if (cal.dataset.pe) return;
  cal.dataset.pe = "1";
  for (const key of cal.querySelectorAll(".cal-head > .icon-btn")) {
    key.classList.remove("icon-btn");
    key.classList.add("key", "sq", "pe-round");
  }
  for (const day of cal.querySelectorAll(".cal-dow")) day.textContent = day.textContent.trim().charAt(0);
  const days = [...cal.querySelectorAll(".cal-day")];
  for (const index of phoneEventSpareWeeks(days.map(day => day.classList.contains("other")))) days[index].classList.add("pe-gone");
  for (const day of days) {
    const count = phoneEventDots(day.querySelectorAll(".cal-event").length, !!day.querySelector(".cal-more"));
    if (!count) continue;
    const dots = document.createElement("span");
    dots.className = "pe-x pe-dots";
    for (let i = 0; i < count; i++) dots.appendChild(document.createElement("i"));
    day.appendChild(dots);
  }
}

function phoneEventsEmpty(empty) {
  if (empty.querySelector(".pe-well")) return;
  const well = document.createElement("span");
  well.className = "pe-x pe-well";
  well.innerHTML = phoneEventIcon("calendar-days");
  empty.prepend(well);
}

function phoneEventsBody() {
  const body = document.getElementById("events-body");
  if (!body) return;
  for (const card of body.querySelectorAll(".event-card")) phoneEventsCard(card);
  for (const cal of body.querySelectorAll(":scope > .cal")) phoneEventsCalendar(cal);
  for (const empty of body.querySelectorAll(":scope > .events-empty")) phoneEventsEmpty(empty);
}

function phoneEventsFields() {
  for (const [id, icon] of Object.entries(PHONE_EVENT_FIELDS)) {
    const input = document.getElementById(id);
    const slot = input && input.parentElement;
    if (!slot || !slot.classList.contains("dfind")) continue;
    if (!slot.querySelector(":scope > .pe-when")) {
      slot.insertAdjacentHTML("afterbegin", phoneEventIcon(icon));
      const when = document.createElement("span");
      when.className = "pe-x pe-when";
      slot.appendChild(when);
    }
    slot.querySelector(":scope > .pe-when").textContent = phoneEventWhen(input.value);
    slot.classList.toggle("pe-on", !!input.value);
  }
  const channel = document.getElementById("event-channel");
  const pick = channel && channel.parentElement;
  if (pick && pick.classList.contains("dsel") && !pick.querySelector(":scope > .pe-lead")) pick.insertAdjacentHTML("afterbegin", phoneEventIcon("volume-2", "pe-lead"));
}

function phoneEventsPicker(input) {
  if (!studioSlim() || typeof input.showPicker !== "function") return;
  try {
    input.showPicker();
  } catch {}
}

function phoneEventsUndress() {
  const create = document.getElementById("event-create");
  if (create && create.classList.contains("pe-key")) {
    create.classList.remove("key", "pe-key");
    create.classList.add("btn-primary", "btn-small");
  }
  for (const node of document.querySelectorAll("#events-layer .pe-x, #modal-event .pe-x")) node.remove();
  for (const slot of document.querySelectorAll("#modal-event .pe-on")) slot.classList.remove("pe-on");
  const layer = document.getElementById("events-layer");
  if (layer && layer.querySelector("#events-body [data-pe]")) {
    if (layer.classList.contains("hidden")) document.getElementById("events-body").replaceChildren();
    else renderEvents();
  }
}

function phoneEventsShown() {
  const layer = document.getElementById("events-layer");
  return studioSlim() && !!layer && !layer.classList.contains("hidden");
}

function phoneEventEditorShown() {
  const backdrop = document.getElementById("modal-backdrop");
  const editor = document.getElementById("modal-event");
  return phoneEventsShown() && !!backdrop && !!editor && !backdrop.classList.contains("hidden") && !editor.classList.contains("hidden");
}

function phoneEventsStep() {
  if (typeof layerStep !== "function") return;
  const events = phoneEventsShown();
  const editor = phoneEventEditorShown();
  const eventsStep = layerStep("ptEvents", events, phoneEventSteps.events, history.state);
  if (eventsStep === "push") history.pushState({ ...(history.state || { pane: phonePane() }), ptEvents: true }, "");
  const editorStep = layerStep("ptEventEditor", editor, phoneEventSteps.editor, history.state);
  if (editorStep === "push") history.pushState({ ...(history.state || { pane: phonePane() }), ptEventEditor: true }, "");
  phoneEventSteps.events = events;
  phoneEventSteps.editor = editor;
  const back = phoneEventStepsBack(editorStep, eventsStep);
  if (back) history.go(-back);
}

function phoneEventsOnBack() {
  const closes = phoneEventBackCloses(history.state, {
    events: phoneEventsShown(),
    editor: phoneEventEditorShown(),
    eventsHeld: phoneEventSteps.events,
    editorHeld: phoneEventSteps.editor,
  });
  if (closes.editor) {
    phoneEventSteps.editor = false;
    hideModals();
  }
  if (closes.events) {
    phoneEventSteps.events = false;
    closeEvents();
  }
}

function phoneEventsSync() {
  phoneEventsStep();
  if (!studioSlim()) {
    phoneEventsUndress();
    return;
  }
  const layer = document.getElementById("events-layer");
  if (layer && !layer.classList.contains("hidden")) {
    phoneEventsHead();
    phoneEventsBody();
  }
  const editor = document.getElementById("modal-event");
  if (editor && !editor.classList.contains("hidden")) phoneEventsFields();
}

function phoneEvents() {
  const layer = document.getElementById("events-layer");
  const body = document.getElementById("events-body");
  const editor = document.getElementById("modal-event");
  if (!layer || !body || !editor) return;
  const sync = () => phoneEventsSync();
  new MutationObserver(sync).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit", "data-look"] });
  new MutationObserver(sync).observe(layer, { attributes: true, attributeFilter: ["class"] });
  new MutationObserver(sync).observe(body, { childList: true });
  new MutationObserver(sync).observe(editor, { attributes: true, attributeFilter: ["class"] });
  const backdrop = document.getElementById("modal-backdrop");
  if (backdrop) new MutationObserver(sync).observe(backdrop, { attributes: true, attributeFilter: ["class"] });
  window.addEventListener("popstate", phoneEventsOnBack);
  for (const id of Object.keys(PHONE_EVENT_FIELDS)) {
    const input = document.getElementById(id);
    if (!input) continue;
    input.addEventListener("click", () => phoneEventsPicker(input));
    input.addEventListener("input", sync);
    input.addEventListener("change", sync);
  }
  sync();
}

if (typeof document !== "undefined" && typeof studioSlim === "function") phoneEvents();

if (typeof module !== "undefined" && module.exports) {
  module.exports = { PHONE_EVENT_DOTS, PHONE_EVENT_FIELDS, phoneEventWhen, phoneEventDots, phoneEventSpareWeeks, phoneEventBackCloses, phoneEventStepsBack };
}
