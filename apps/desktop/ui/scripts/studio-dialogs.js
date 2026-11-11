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

const MAKING_DIALOGS = [
  "modal-create-channel",
  "modal-add-server",
  "modal-username",
  "modal-create-server",
  "modal-event",
  "modal-add-bot",
  "modal-poll",
  "modal-forum-post",
  "modal-sound",
  "modal-catalog",
];

const MAKING_KEYS = {
  "create-server-open": "key primary",
  "join-server-save": "key",
  "username-save": "key primary",
  "add-bot-invite-copy": "key sm",
  "poll-add": "key sm",
  "sound-trim-play": "key sq",
  "sound-emoji": "key sq",
  "catalog-close": "key sq",
  "catalog-upload": "key sm",
  "catalog-add": "key sm primary",
};

const MAKING_ROW_KEYS = [
  ["add-bot-commands", ".bot-code-row > .bot-copy", "key sm"],
  ["add-bot-bridge", ".bot-code-row > .bot-copy", "key sm"],
  ["poll-rows", ".poll-row > .poll-row-remove", "key sq"],
];

const MAKING_FIELD_ICONS = {
  "join-ticket-input": "link",
  "username-input": "at-sign",
  "event-start": "calendar-days",
  "event-end": "calendar-days",
  "catalog-search": "search",
};

const MAKING_SELECTS = ["event-channel", "poll-duration"];

const STUDIO_KEY_NAMES = ["key", "sm", "sq", "primary"];

function studioKeyNames(classes, roomy) {
  const wanted = roomy ? String(classes || "").split(" ") : [];
  return Object.fromEntries(STUDIO_KEY_NAMES.map(name => [name, wanted.includes(name)]));
}

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
  modal.classList.toggle("make", roomy && MAKING_DIALOGS.includes(modal.id));
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

function studioFieldIcon(input, icon = "search", kind = "dfind") {
  if (input.parentElement && input.parentElement.classList.contains(kind)) return;
  const wrap = document.createElement("span");
  wrap.className = kind;
  input.before(wrap);
  if (typeof studioIconMarkup === "function") wrap.insertAdjacentHTML("afterbegin", studioIconMarkup(icon));
  wrap.appendChild(input);
}

function studioKeyDress(button, classes, roomy) {
  for (const [name, on] of Object.entries(studioKeyNames(classes, roomy))) studioToggle(button, name, on);
}

function studioMakingKeys() {
  const roomy = document.documentElement.dataset.fit !== "slim";
  for (const [id, classes] of Object.entries(MAKING_KEYS)) {
    const key = document.getElementById(id);
    if (key) studioKeyDress(key, classes, roomy);
  }
  for (const [id, selector, classes] of MAKING_ROW_KEYS) {
    const list = document.getElementById(id);
    if (!list) continue;
    for (const key of list.querySelectorAll(selector)) studioKeyDress(key, classes, roomy);
  }
}

function studioMakingParts() {
  for (const [id, icon] of Object.entries(MAKING_FIELD_ICONS)) {
    const input = document.getElementById(id);
    if (input) studioFieldIcon(input, icon);
  }
  for (const select of MAKING_SELECTS.map(id => document.getElementById(id)).filter(Boolean)) studioFieldIcon(select, "chevron-down", "dsel");
  const username = document.getElementById("username-input");
  const wrap = username && username.parentElement;
  if (wrap && wrap.classList.contains("dfind") && !wrap.previousElementSibling?.classList.contains("dlabel")) {
    const label = document.createElement("label");
    label.className = "field-label dlabel";
    label.htmlFor = username.id;
    label.textContent = username.placeholder;
    wrap.before(label);
  }
  if (typeof studioIconMarkup !== "function") return;
  for (const kind of document.querySelectorAll("#create-channel-kinds > .kind-option")) {
    if (!kind.querySelector(":scope > .tick")) kind.insertAdjacentHTML("beforeend", studioIconMarkup("check", "tick"));
  }
  const add = document.getElementById("poll-add");
  if (add && !add.querySelector(":scope > .i")) add.insertAdjacentHTML("afterbegin", studioIconMarkup("plus"));
  const fold = document.querySelector("#add-bot-port > summary");
  if (fold && !fold.querySelector(":scope > .i")) fold.insertAdjacentHTML("afterbegin", studioIconMarkup("chevron-right"));
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
    if (key) studioKeyDress(key, classes, roomy);
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
  const modals = [...STUDIO_DIALOGS, ...JOIN_DIALOGS, ...MODERATE_DIALOGS, ...PEOPLE_DIALOGS, ...MAKING_DIALOGS].map(id => document.getElementById(id)).filter(Boolean);
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
    studioMakingKeys();
  };
  const keys = new MutationObserver(() => { for (const grid of grids) studioLengthKeys(grid); });
  for (const grid of grids) keys.observe(grid, { childList: true });
  for (const input of PEOPLE_SEARCHES.map(id => document.getElementById(id)).filter(Boolean)) studioFieldIcon(input);
  const lists = PEOPLE_LISTS.map(id => document.getElementById(id)).filter(Boolean);
  const rows = new MutationObserver(() => { for (const list of lists) studioPeopleRows(list); });
  for (const list of lists) rows.observe(list, { childList: true, subtree: true, attributes: true, attributeFilter: ["class", "disabled"] });
  studioMakingParts();
  const keyLists = MAKING_ROW_KEYS.map(([id]) => document.getElementById(id)).filter(Boolean);
  const making = new MutationObserver(studioMakingKeys);
  for (const list of keyLists) making.observe(list, { childList: true, subtree: true });
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
  module.exports = { STUDIO_DIALOGS, DIALOG_BLUE_ACTIONS, dialogAnswerClasses, JOIN_DIALOGS, JOIN_ANSWERS, joinAnswerClasses, joinTileServer, dialogJustOpened, MODERATE_DIALOGS, leavingTitleIcon, PEOPLE_DIALOGS, PEOPLE_LISTS, PEOPLE_SEARCHES, INVITE_KEYS, inviteRowKey, MAKING_DIALOGS, MAKING_KEYS, MAKING_ROW_KEYS, MAKING_FIELD_ICONS, MAKING_SELECTS, studioKeyNames };
}
