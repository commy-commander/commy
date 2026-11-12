const PICKER_TABS = [
  { kind: "emojis", label: "Emoji" },
  { kind: "stickers", label: "Stickers" },
  { kind: "emoticons", label: "Emoticons" },
  { kind: "gifs", label: "GIFs" },
];
const GIF_OFF_WORDS = "Your account server does not offer GIF search.";

const phonePicker = { step: false, shown: false, homes: null, drag: null };

function pickerTabFor(gifOpen, kind) {
  return gifOpen ? "gifs" : PICKER_TABS.some(tab => tab.kind === kind) ? kind : "emojis";
}

function gifSearchOff(text) {
  return String(text || "").trim().startsWith(GIF_OFF_WORDS);
}

function pickerEmojiOpen() {
  return !$("emoji-picker").classList.contains("hidden");
}

function pickerGifOpen() {
  return !$("gif-picker").classList.contains("hidden");
}

function pickerShown() {
  return studioSlim() && (pickerEmojiOpen() || pickerGifOpen());
}

function pickerKindNow() {
  const tab = $("picker-tabs").querySelector("button.active");
  return tab ? tab.dataset.kind : "emojis";
}

function buildPickerSheet() {
  const box = el("div", "pt-picker");
  box.id = "pt-picker";
  box.hidden = true;
  const scrim = el("div", "pt-picker-scrim");
  const sheet = el("section", "pt-picker-sheet");
  sheet.id = "pt-picker-sheet";
  sheet.setAttribute("role", "dialog");
  sheet.setAttribute("aria-label", "Emoji");
  const grab = el("div", "pt-picker-grab");
  grab.setAttribute("aria-hidden", "true");
  const tabs = el("div", "dtabs pt-picker-tabs");
  tabs.id = "pt-picker-tabs";
  tabs.setAttribute("role", "tablist");
  for (const item of PICKER_TABS) {
    const tab = el("button", "", item.label);
    tab.type = "button";
    tab.dataset.kind = item.kind;
    tab.setAttribute("role", "tab");
    tab.setAttribute("aria-selected", "false");
    tab.onclick = () => choosePickerTab(item.kind);
    tabs.appendChild(tab);
  }
  sheet.append(grab, tabs);
  box.append(scrim, sheet);
  for (const part of [scrim, grab, tabs]) part.addEventListener("mousedown", e => e.stopPropagation());
  scrim.onclick = closePickerSheet;
  wirePickerGrab(grab, sheet);
  $("app").appendChild(box);
}

function choosePickerTab(kind) {
  if (kind === "gifs") {
    if (!pickerGifOpen()) openGifPicker("");
    return;
  }
  if (pickerGifOpen()) closeGifPicker();
  if (!pickerEmojiOpen()) $("emoji-btn").click();
  const tab = $("picker-tabs").querySelector(`button[data-kind="${kind}"]`);
  if (tab && pickerKindNow() !== kind) tab.click();
}

function closePickerSheet() {
  $("emoji-picker").classList.add("hidden");
  closeGifPicker();
}

function dressPickers() {
  const slim = studioSlim();
  const sheet = $("pt-picker-sheet");
  if (slim && !phonePicker.homes) {
    const emoji = $("emoji-picker");
    const gif = $("gif-picker");
    phonePicker.homes = [[emoji, emoji.parentNode, emoji.nextSibling], [gif, gif.parentNode, gif.nextSibling]];
    sheet.append(emoji, gif);
  } else if (!slim && phonePicker.homes) {
    for (const [node, parent, next] of phonePicker.homes) parent.insertBefore(node, next && next.parentNode === parent ? next : null);
    phonePicker.homes = null;
  }
  syncPickerSheet();
}

function paintPickerTabs() {
  const current = pickerTabFor(pickerGifOpen(), pickerKindNow());
  const gifKey = $("gif-btn");
  for (const tab of $("pt-picker-tabs").querySelectorAll("button")) {
    const on = tab.dataset.kind === current;
    tab.classList.toggle("on", on);
    tab.setAttribute("aria-selected", String(on));
    if (tab.dataset.kind === "gifs") tab.hidden = gifKey.classList.contains("hidden") && !pickerGifOpen();
  }
  const label = PICKER_TABS.find(tab => tab.kind === current);
  $("pt-picker-sheet").setAttribute("aria-label", label ? label.label : "Emoji");
}

function markGifOff() {
  const note = $("gif-grid").querySelector(":scope > .gif-note");
  $("gif-picker").classList.toggle("pt-gif-off", Boolean(note) && gifSearchOff(note.textContent));
}

function syncPickerSheet() {
  const open = pickerShown();
  const opening = open && !phonePicker.shown;
  phonePicker.shown = open;
  $("pt-picker").hidden = !open;
  if (!open) $("pt-picker-sheet").style.translate = "";
  if (opening && pickerEmojiOpen()) {
    if (pickerKindNow() !== "emojis") $("picker-tabs").querySelector('button[data-kind="emojis"]').click();
    else $("composer-emoji-browser").querySelector(".eb-body")?.dispatchEvent(new Event("scroll"));
  }
  paintPickerTabs();
  const step = layerStep("ptPicker", open, phonePicker.step, history.state);
  if (step === "push") history.pushState({ ...(history.state || { pane: phonePane() }), ptPicker: true }, "");
  phonePicker.step = open;
  if (step === "back") history.back();
}

function wirePickerGrab(grab, sheet) {
  grab.addEventListener("pointerdown", e => {
    if (!pickerShown()) return;
    phonePicker.drag = { id: e.pointerId, y: e.clientY, dy: 0 };
    grab.setPointerCapture(e.pointerId);
    sheet.classList.add("pt-dragging");
  });
  grab.addEventListener("pointermove", e => {
    const drag = phonePicker.drag;
    if (!drag || drag.id !== e.pointerId) return;
    drag.dy = e.clientY - drag.y;
    sheet.style.translate = `0 ${sheetPull(drag.dy)}px`;
  });
  const end = cancelled => e => {
    const drag = phonePicker.drag;
    if (!drag || drag.id !== e.pointerId) return;
    phonePicker.drag = null;
    sheet.classList.remove("pt-dragging");
    if (!cancelled && sheetCloses(drag.dy)) closePickerSheet();
    else sheet.style.translate = "";
  };
  grab.addEventListener("pointerup", end(false));
  grab.addEventListener("pointercancel", end(true));
}

function wirePhonePicker() {
  if (!$("emoji-picker") || !$("gif-picker") || !$("app")) return;
  buildPickerSheet();
  const watch = (node, then) => new MutationObserver(then).observe(node, { attributes: true, attributeFilter: ["class"] });
  watch($("emoji-picker"), syncPickerSheet);
  watch($("gif-picker"), syncPickerSheet);
  watch($("gif-btn"), paintPickerTabs);
  for (const tab of $("picker-tabs").querySelectorAll("button")) watch(tab, paintPickerTabs);
  new MutationObserver(markGifOff).observe($("gif-grid"), { childList: true });
  new MutationObserver(dressPickers).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit", "data-look"] });
  window.addEventListener("popstate", () => {
    if ((history.state && history.state.ptPicker) || !pickerShown()) return;
    phonePicker.step = false;
    closePickerSheet();
  });
  dressPickers();
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { PICKER_TABS, GIF_OFF_WORDS, pickerTabFor, gifSearchOff };
} else {
  wirePhonePicker();
}
