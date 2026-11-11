const STUDIO_SAVED_CLOSE_KEY = "key sq";

function studioSavedEntry(id, store) {
  if (!id || !store) return null;
  const saved = (store.saved || []).find(entry => entry && entry.message === id);
  if (saved) return saved;
  const reminder = (store.reminders || []).find(r => r && r.entry && r.entry.message === id);
  return reminder ? reminder.entry : null;
}

function studioSavedInk(entry, server, ink) {
  if (!entry || !server || !entry.server || entry.server !== server.id) return "";
  const member = (server.members || []).find(m => m && m.id === entry.author_id);
  return member && member.color ? ink(member.color, true) : "";
}

function studioSavedRows(list) {
  const roomy = document.documentElement.dataset.fit !== "slim";
  const server = typeof state !== "undefined" ? state.currentServer : null;
  for (const row of list.querySelectorAll(":scope > .saved-row")) {
    const author = row.querySelector(".saved-author");
    if (!author) continue;
    const color = roomy && typeof roleInk === "function" ? studioSavedInk(studioSavedEntry(row.dataset.id, savedState), server, roleInk) : "";
    if (author.style.getPropertyValue("--saved-ink") === color) continue;
    if (color) author.style.setProperty("--saved-ink", color);
    else author.style.removeProperty("--saved-ink");
  }
}

function studioSaved() {
  const list = document.getElementById("saved-list");
  const close = document.getElementById("saved-close");
  if (!list || !close) return;
  const dress = () => {
    if (typeof studioKeyDress === "function") studioKeyDress(close, STUDIO_SAVED_CLOSE_KEY, document.documentElement.dataset.fit !== "slim");
    studioSavedRows(list);
  };
  dress();
  new MutationObserver(() => studioSavedRows(list)).observe(list, { childList: true });
  new MutationObserver(dress).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit"] });
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { STUDIO_SAVED_CLOSE_KEY, studioSavedEntry, studioSavedInk };
}
