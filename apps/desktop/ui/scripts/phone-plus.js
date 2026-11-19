const PLUS_KEYS = [
  { id: "file", get label() { return t("ui.file"); }, icon: "file", press: "cm-file", files: true },
  { id: "image", get label() { return t("ui.image"); }, icon: "image", press: "cm-photo", files: true },
  { id: "emoji", get label() { return t("ui.emoji"); }, icon: "smile", press: "emoji-btn" },
  { id: "poll", get label() { return t("ui.poll"); }, icon: "chart-bar-big", press: "cm-poll", poll: true },
  { id: "voice", get label() { return t("settings.tab.voice"); }, icon: "mic", press: "cm-voice", voice: true, get title() { return t("ui.record-a-voice-message"); } },
];
const FIELD_KEYS = [
  { press: "text-style-btn", icon: "a-large-small" },
  { press: "gif-btn", icon: "gif" },
];

function plusBar() {
  return document.querySelector(".main > .composer");
}

function buildPlusKeys() {
  const keys = studioNode("div", "pt-plus");
  keys.id = "pt-plus";
  keys.hidden = true;
  keys.setAttribute("role", "group");
  keys.setAttribute("aria-label", t("ui.attach"));
  for (const item of PLUS_KEYS) {
    const key = studioNode("button", "pt-plus-k", `<span class="ic"><i data-i="${item.icon}"></i></span><span class="lb">${item.label}</span>`);
    key.type = "button";
    key.id = `pt-plus-${item.id}`;
    key.tabIndex = -1;
    if (item.title) {
      key.title = item.title;
      key.setAttribute("aria-label", item.title);
    }
    key.onclick = () => {
      key.blur();
      foldChatBar(false);
      setTimeout(() => $(item.press).click(), 0);
    };
    keys.appendChild(key);
  }
  plusBar().appendChild(keys);
}

function foldChatBar(open) {
  const bar = plusBar();
  if (!open && !bar.classList.contains("pt-plus-open")) return;
  if (open) {
    if (!$("pt-plus")) buildPlusKeys();
    const files = state.view !== "server" || canAttachHere();
    const voice = (state.view !== "server" || canSendVoiceHere()) && voiceRec.target === null;
    const poll = state.view === "server" && canSendPolls(state.currentServer);
    for (const item of PLUS_KEYS) $(`pt-plus-${item.id}`).hidden = (item.files && !files) || (item.poll && !poll) || (item.voice && !voice);
    closeFloating();
    $("message-input").blur();
  }
  bar.classList.toggle("pt-plus-open", open);
  $("attach-btn").setAttribute("aria-expanded", String(open));
  $("attach-btn").setAttribute("aria-label", open ? t("settings.close") : t("ui.attach"));
  for (const key of bar.querySelectorAll(".pt-plus-k")) key.tabIndex = open ? 0 : -1;
  $("message-input").tabIndex = open ? -1 : 0;
  $("send-btn").tabIndex = open ? -1 : 0;
}

function dressFieldKeys() {
  for (const item of FIELD_KEYS) {
    const icon = studioNode("span", "pt-fk", `<i data-i="${item.icon}"></i>`);
    icon.hidden = true;
    $(item.press).appendChild(icon);
  }
}

function markTextStyle() {
  const pop = $("text-style-pop");
  const key = $("text-style-btn");
  const open = studioSlim() && !pop.classList.contains("hidden");
  if (!studioSlim()) {
    key.removeAttribute("aria-expanded");
    return;
  }
  key.setAttribute("aria-expanded", String(open));
  if (open) pop.style.setProperty("--pt-ts-bottom", `${Math.round(window.innerHeight - plusBar().getBoundingClientRect().top + 8)}px`);
}

function wirePhonePlus() {
  if (!$("attach-btn") || !plusBar()) return;
  dressFieldKeys();
  document.addEventListener("click", e => {
    if (!studioSlim() || !e.target.closest || !e.target.closest("#attach-btn")) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    foldChatBar(!plusBar().classList.contains("pt-plus-open"));
  }, true);
  const shut = () => foldChatBar(false);
  new MutationObserver(shut).observe($("app"), { attributes: true, attributeFilter: ["data-pane"] });
  new MutationObserver(shut).observe($("message-input"), { attributes: true, attributeFilter: ["placeholder"] });
  new MutationObserver(markTextStyle).observe($("text-style-pop"), { attributes: true, attributeFilter: ["class"] });
  window.addEventListener("popstate", shut);
}

wirePhonePlus();
