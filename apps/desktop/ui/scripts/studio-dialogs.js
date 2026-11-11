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

function answerKind(button) {
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

const DIALOG_KEY_CLASSES = ["key", "cream", "danger", "primary", "third"];

function studioDialogDress(modal) {
  const roomy = document.documentElement.dataset.fit !== "slim";
  modal.classList.toggle("dialog", roomy);
  modal.classList.toggle("slab", roomy);
  const actions = modal.querySelector(":scope > .modal-actions");
  if (!actions) return;
  actions.classList.toggle("answers", roomy);
  const buttons = [...actions.querySelectorAll(":scope > button")];
  const classes = dialogAnswerClasses(buttons.map(answerKind));
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

function studioDialogs() {
  studioDialogParts();
  const modals = STUDIO_DIALOGS.map(id => document.getElementById(id)).filter(Boolean);
  const dress = () => { for (const modal of modals) studioDialogDress(modal); };
  dress();
  new MutationObserver(dress).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit"] });
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { STUDIO_DIALOGS, dialogAnswerClasses };
}
