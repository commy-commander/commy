const PHONE_MENU_ICONS = {
  "sm-read": "check-check",
  "sm-notify": "bell",
  "sm-invite": "user-plus",
  "sm-pins": "pin",
  "sm-events": "calendar-days",
  "sm-rules": "scroll-text",
  "sm-roles": "tag",
  "sm-onboarding": "list",
  "sm-join-sound": "music",
  "pt-sm-add": "plus",
  "sm-settings": "settings",
  "sm-ungroup": "folder",
  "sm-leave": "log-out",
  "chm-read": "check-check",
  "chm-voice-join": "volume-2",
  "chm-voice-invite": "user-plus",
  "chm-activities": "gamepad-2",
  "chm-pin": "pin",
  "chm-voice-status": "pencil",
  "chm-settings": "settings",
};
const PHONE_MENU_SWITCH_WORDS = { "sm-mute": "Mute Server" };
const PHONE_MENUS = ["server-menu", "channel-menu"];

function phoneMenuKind(button) {
  if (button.classList.contains("ctx-check")) return "switch";
  if (button.classList.contains("ctx-radio") && button.dataset.level) return "pick";
  return "row";
}

function dressPhoneMenuRow(button) {
  const kind = phoneMenuKind(button);
  if (!button.querySelector(":scope > .l")) {
    const words = PHONE_MENU_SWITCH_WORDS[button.id] || button.textContent;
    button.replaceChildren();
    const icon = PHONE_MENU_ICONS[button.id];
    if (icon) button.insertAdjacentHTML("beforeend", studioIconMarkup(icon));
    button.appendChild(el("span", "l", words));
    if (kind === "pick") button.insertAdjacentHTML("beforeend", studioIconMarkup("check", "tick"));
    if (kind === "switch") button.appendChild(el("i", "tog"));
  }
  button.classList.add("arow");
  if (kind === "pick") button.setAttribute("role", "menuitemradio");
  if (kind === "switch") button.setAttribute("role", "menuitemcheckbox");
  if (kind !== "row") button.setAttribute("aria-checked", String(button.classList.contains("checked")));
}

function undressPhoneMenuRow(button) {
  const words = button.querySelector(":scope > .l");
  if (words) button.textContent = words.textContent;
  button.classList.remove("arow");
  button.removeAttribute("role");
  button.removeAttribute("aria-checked");
}

function dressPhoneMenu(menu) {
  if (menu.classList.contains("hidden")) return;
  const slim = studioSlim();
  for (const button of menu.querySelectorAll(":scope > button")) {
    if (slim) dressPhoneMenuRow(button);
    else undressPhoneMenuRow(button);
  }
}

function wirePhoneMenus() {
  for (const id of PHONE_MENUS) {
    const menu = $(id);
    if (!menu) continue;
    const watch = new MutationObserver(() => {
      dressPhoneMenu(menu);
      watch.takeRecords();
    });
    watch.observe(menu, { attributes: true, attributeFilter: ["class"], childList: true, subtree: true, characterData: true });
  }
}

wirePhoneMenus();
