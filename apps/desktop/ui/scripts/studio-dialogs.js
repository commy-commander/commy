const STUDIO_DIALOGS = [
  "modal-delete-account",
  "modal-leave-server",
  "modal-delete-server",
  "modal-transfer-owner",
  "modal-block",
  "modal-delete-message",
  "modal-clear-cache",
  "modal-clear-all",
  "modal-move-data",
  "modal-entry-start-over",
  "modal-stream-deck",
  "modal-nickname",
  "modal-voice-status",
  "modal-rename-folder",
  "modal-add-friend",
  "modal-tray",
  "modal-choose-roles",
  "modal-emoticon",
  "modal-drive-item",
  "modal-report",
  "modal-delete-channel",
];

const DIALOG_BLUE_ACTIONS = ["report-go"];

const JOIN_DIALOGS = [
  "modal-invite-link",
  "modal-join-wait",
  "modal-server-welcome",
  "modal-onboarding",
  "modal-rules",
  "modal-server-info",
];

const JOIN_ANSWERS = {
  "invite-link-cancel": "key",
  "invite-link-join": "key primary",
  "join-wait-cancel": "key out-k",
  "join-wait-send": "key",
  "join-wait-close": "key primary",
  "sw-later": "key",
  "sw-next": "key primary",
  "ob-later": "key",
  "ob-back": "key",
  "ob-next": "key primary",
  "rules-close": "key",
  "rules-accept": "key primary",
  "si-leave": "key out-k",
  "si-answers": "key",
  "si-close": "key primary",
};

const MODERATE_DIALOGS = ["modal-moderate", "modal-timeout", "modal-leaving"];

const PEOPLE_DIALOGS = ["modal-invite", "modal-forward", "modal-voice-invite", "modal-new-dm", "modal-group"];

const PEOPLE_LISTS = ["invite-people", "forward-list", "voice-invite-list", "new-dm-list", "group-list"];

const PEOPLE_SEARCHES = ["invite-friend-search", "forward-search", "voice-invite-search", "new-dm-search", "group-search"];

const INVITE_KEYS = { "invite-link-copy": "key primary", "invite-copy": "key" };

function inviteRowKey(text, disabled) {
  return String(text || "").trim() === "Sent" && disabled ? "sent" : "key sm";
}

const LEAVING_TITLE_ICONS = { warn: "triangle-alert", block: "ban" };

function leavingTitleIcon(classes) {
  const list = [...(classes || [])];
  if (list.includes("block")) return LEAVING_TITLE_ICONS.block;
  if (list.includes("warn")) return LEAVING_TITLE_ICONS.warn;
  return null;
}

function joinAnswerClasses(ids) {
  return ids.map(id => JOIN_ANSWERS[id] || "key");
}

function joinTileServer(modalId, current, waiting) {
  if (modalId === "modal-invite-link") return null;
  if (modalId === "modal-join-wait") return waiting || null;
  return current || null;
}

function answerKind(button) {
  if (DIALOG_BLUE_ACTIONS.includes(button.id)) return "primary";
  if (button.classList.contains("btn-danger")) return "danger";
  if (button.classList.contains("btn-primary")) return "primary";
  return "plain";
}

function dialogAnswerClasses(kinds) {
  let action = -1;
  kinds.forEach((kind, index) => {
    if (kind !== "plain") action = index;
  });
  const red = action >= 0 && kinds[action] === "danger";
  return kinds.map((kind, index) => {
    if (index === action) return red ? "key danger" : "key primary";
    if (kind !== "plain") return "key";
    if (index === 0) return red ? "key cream" : "key";
    return "key third";
  });
}

const DIALOG_KEY_CLASSES = ["key", "cream", "danger", "primary", "third", "out-k"];

function studioDialogDress(modal) {
  const roomy = document.documentElement.dataset.fit !== "slim";
  const join = JOIN_DIALOGS.includes(modal.id);
  modal.classList.toggle("dialog", roomy);
  modal.classList.toggle("slab", roomy);
  modal.classList.toggle("join", roomy && join);
  for (const box of modal.querySelectorAll(":scope > .report-also-block > input[type='checkbox'], :scope > .leaving-trust > input[type='checkbox']")) box.classList.toggle("switch", roomy);
  const actions = modal.querySelector(":scope > .modal-actions");
  if (!actions) return;
  actions.classList.toggle("answers", roomy);
  const buttons = [...actions.querySelectorAll(":scope > button")];
  const classes = join ? joinAnswerClasses(buttons.map(button => button.id)) : dialogAnswerClasses(buttons.map(answerKind));
  actions.classList.toggle("spread", roomy && classes.some(name => name.includes("out-k")));
  buttons.forEach((button, index) => {
    const wanted = roomy ? classes[index].split(" ") : [];
    for (const name of DIALOG_KEY_CLASSES) {
      const on = wanted.includes(name);
      if (button.classList.contains(name) !== on) button.classList.toggle(name, on);
    }
  });
}

function studioDialogParts() {
  if (typeof studioIconMarkup !== "function") return;
  const callout = document.querySelector("#modal-delete-channel > .danger-callout");
  if (callout && !callout.querySelector(":scope > .i")) callout.insertAdjacentHTML("afterbegin", studioIconMarkup("triangle-alert"));
  for (const reason of document.querySelectorAll("#report-reasons > label")) {
    if (!reason.querySelector(":scope > .tick")) reason.insertAdjacentHTML("beforeend", studioIconMarkup("check", "tick"));
  }
}

function studioLengthKeys(grid) {
  const roomy = document.documentElement.dataset.fit !== "slim";
  for (const choice of grid.querySelectorAll(":scope > .timeout-choice")) {
    const down = roomy && choice.classList.contains("active");
    for (const name of ["key", "latch"]) {
      if (choice.classList.contains(name) !== roomy) choice.classList.toggle(name, roomy);
    }
    if (choice.classList.contains("is-down") !== down) choice.classList.toggle("is-down", down);
    if (!choice.querySelector(":scope > .led")) {
      const led = document.createElement("i");
      led.className = "led";
      choice.appendChild(led);
    }
  }
}

function studioToggle(node, name, on) {
  if (node.classList.contains(name) !== on) node.classList.toggle(name, on);
}

function studioPeopleRows(list) {
  const roomy = document.documentElement.dataset.fit !== "slim";
  for (const row of list.querySelectorAll(":scope > .person-row")) {
    const chip = row.querySelector(":scope > .server-chip");
    if (chip && row.dataset.server && typeof serverTint === "function") chip.style.setProperty("--server-tint", serverTint(row.dataset.server));
    if (row.classList.contains("new-group") && roomy && typeof studioIconMarkup === "function" && !row.querySelector(":scope > .go")) row.insertAdjacentHTML("beforeend", studioIconMarkup("chevron-right", "go"));
    const pick = row.querySelector(":scope > .pick");
    if (pick && roomy && typeof studioIconMarkup === "function" && !pick.querySelector(".i")) pick.insertAdjacentHTML("afterbegin", studioIconMarkup("check", "tick"));
    for (const button of row.querySelectorAll(":scope > button")) {
      const kind = roomy ? inviteRowKey(button.textContent, button.disabled) : "";
      studioToggle(button, "key", kind === "key sm");
      studioToggle(button, "sm", kind === "key sm");
      studioToggle(button, "sent", kind === "sent");
      if (kind === "sent" && typeof studioIconMarkup === "function" && !button.querySelector(":scope > .i")) button.insertAdjacentHTML("afterbegin", studioIconMarkup("check"));
    }
  }
}

function studioPeopleSearch(input) {
  if (input.parentElement && input.parentElement.classList.contains("dfind")) return;
  const wrap = document.createElement("span");
  wrap.className = "dfind";
  input.before(wrap);
  if (typeof studioIconMarkup === "function") wrap.insertAdjacentHTML("afterbegin", studioIconMarkup("search"));
  wrap.appendChild(input);
}

function studioInviteParts() {
  const roomy = document.documentElement.dataset.fit !== "slim";
  const expiry = document.getElementById("invite-expiry");
  if (expiry) {
    for (const choice of expiry.querySelectorAll(":scope > button")) {
      studioToggle(choice, "key", roomy);
      studioToggle(choice, "latch", roomy);
      studioToggle(choice, "is-down", roomy && choice.classList.contains("active"));
      if (!choice.querySelector(":scope > .led")) {
        const led = document.createElement("i");
        led.className = "led";
        choice.appendChild(led);
      }
    }
  }
  for (const [id, classes] of Object.entries(INVITE_KEYS)) {
    const key = document.getElementById(id);
    if (!key) continue;
    const wanted = roomy ? classes.split(" ") : [];
    for (const name of ["key", "primary"]) studioToggle(key, name, wanted.includes(name));
  }
}

function studioLeavingTitle(modal) {
  const title = modal.querySelector(":scope > h2");
  if (!title) return;
  const roomy = document.documentElement.dataset.fit !== "slim";
  const name = roomy ? leavingTitleIcon(modal.classList) : null;
  const icon = title.querySelector(":scope > .i");
  if (icon && (!name || icon.dataset.icon !== name)) icon.remove();
  if (name && typeof studioIconMarkup === "function" && !title.querySelector(":scope > .i")) title.insertAdjacentHTML("afterbegin", studioIconMarkup(name));
}

function dialogJustOpened(oldClasses, hiddenNow) {
  return !hiddenNow && String(oldClasses || "").split(/\s+/).includes("hidden");
}

function studioJoinParts(modal) {
  if (modal.classList.contains("hidden") || document.documentElement.dataset.fit === "slim") return;
  const icon = modal.querySelector(".sw-icon");
  if (icon) {
    const current = typeof state !== "undefined" && state.currentServer ? state.currentServer.id : null;
    const waiting = typeof joinWaitShown !== "undefined" ? joinWaitShown : null;
    const id = joinTileServer(modal.id, current, waiting);
    if (id) icon.style.setProperty("--server-tint", serverTint(id));
    else icon.style.removeProperty("--server-tint");
  }
  if (typeof studioIconMarkup !== "function") return;
  const warning = modal.querySelector(":scope > .invite-link-warning");
  if (warning && !warning.querySelector(":scope > .i")) warning.insertAdjacentHTML("afterbegin", studioIconMarkup("triangle-alert"));
  [...modal.querySelectorAll("#sw-steps > .sw-step")].forEach((step, index) => {
    if (step.querySelector(":scope > b")) return;
    step.insertAdjacentHTML("afterbegin", `<b>${index + 1}</b>${studioIconMarkup("check")}`);
  });
}

function studioDialogs() {
  studioDialogParts();
  const modals = [...STUDIO_DIALOGS, ...JOIN_DIALOGS, ...MODERATE_DIALOGS, ...PEOPLE_DIALOGS].map(id => document.getElementById(id)).filter(Boolean);
  const joins = modals.filter(modal => JOIN_DIALOGS.includes(modal.id));
  const grids = ["moderate-lengths", "timeout-choices"].map(id => document.getElementById(id)).filter(Boolean);
  const leaving = document.getElementById("modal-leaving");
  const dress = () => {
    for (const modal of modals) studioDialogDress(modal);
    for (const modal of joins) studioJoinParts(modal);
    for (const grid of grids) studioLengthKeys(grid);
    if (leaving) studioLeavingTitle(leaving);
    for (const list of PEOPLE_LISTS.map(id => document.getElementById(id)).filter(Boolean)) studioPeopleRows(list);
    studioInviteParts();
  };
  const keys = new MutationObserver(() => { for (const grid of grids) studioLengthKeys(grid); });
  for (const grid of grids) keys.observe(grid, { childList: true });
  for (const input of PEOPLE_SEARCHES.map(id => document.getElementById(id)).filter(Boolean)) studioPeopleSearch(input);
  const lists = PEOPLE_LISTS.map(id => document.getElementById(id)).filter(Boolean);
  const rows = new MutationObserver(() => { for (const list of lists) studioPeopleRows(list); });
  for (const list of lists) rows.observe(list, { childList: true, subtree: true, attributes: true, attributeFilter: ["class", "disabled"] });
  const expiry = document.getElementById("invite-expiry");
  if (expiry) new MutationObserver(studioInviteParts).observe(expiry, { subtree: true, attributes: true, attributeFilter: ["class"] });
  dress();
  new MutationObserver(dress).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit"] });
  const shown = new MutationObserver(records => {
    for (const record of records) {
      if (dialogJustOpened(record.oldValue, record.target.classList.contains("hidden"))) record.target.classList.add("quiet");
    }
    for (const modal of new Set(records.map(record => record.target))) {
      if (joins.includes(modal)) studioJoinParts(modal);
      if (MODERATE_DIALOGS.includes(modal.id)) studioDialogDress(modal);
      if (modal === leaving) studioLeavingTitle(modal);
    }
  });
  for (const modal of modals) shown.observe(modal, { attributes: true, attributeFilter: ["class"], attributeOldValue: true });
  document.addEventListener("keydown", () => {
    for (const modal of modals) modal.classList.remove("quiet");
  }, true);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { STUDIO_DIALOGS, DIALOG_BLUE_ACTIONS, dialogAnswerClasses, JOIN_DIALOGS, JOIN_ANSWERS, joinAnswerClasses, joinTileServer, dialogJustOpened, MODERATE_DIALOGS, leavingTitleIcon, PEOPLE_DIALOGS, PEOPLE_LISTS, PEOPLE_SEARCHES, INVITE_KEYS, inviteRowKey };
}
