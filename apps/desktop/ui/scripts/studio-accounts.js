function studioAccountsWindow() {
  const modal = document.getElementById("modal-accounts");
  if (!modal || modal.classList.contains("slab")) return;
  modal.classList.add("slab");
  const grab = document.createElement("span");
  grab.className = "set-grab";
  grab.setAttribute("aria-hidden", "true");
  modal.prepend(grab);
  const add = document.getElementById("account-add");
  if (add) add.classList.add("key");
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { studioAccountsWindow };
}
