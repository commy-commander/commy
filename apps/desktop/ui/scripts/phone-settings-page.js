const phoneSetPage = { moved: [] };

function phoneSetMove(node, className, place) {
  phoneSetPage.moved.push({ node, className, parent: node.parentNode, next: node.nextSibling });
  node.classList.add(className);
  place(node);
}

function phoneSetRestore() {
  while (phoneSetPage.moved.length) {
    const { node, className, parent, next } = phoneSetPage.moved.pop();
    node.classList.remove(className);
    parent.insertBefore(node, next);
  }
  for (const face of document.querySelectorAll(".account-card .set-me")) face.remove();
  for (const head of document.querySelectorAll(".settings-layer .set-head")) head.remove();
}

function phoneSetServerHead() {
  const card = document.querySelector("#server-settings .overview-card");
  const pane = card && card.parentElement;
  if (!pane || !pane.classList.contains("set-pane") || pane.querySelector(":scope > .set-head")) return;
  pane.prepend(el("h2", "settings-subhead set-head", "Server"));
}

function phoneSetLift(page) {
  const first = page.querySelector(":scope > .set-pane");
  const lead = first && first.querySelector(":scope > .setting-sub.expr-intro:first-child");
  if (lead) phoneSetMove(lead, "set-lead", node => first.before(node));
  for (const pane of page.querySelectorAll(":scope > .set-pane")) {
    let after = pane;
    for (const wrap of pane.querySelectorAll(":scope > div:not(.settings-card):has(> .settings-subhead)")) {
      const at = after;
      phoneSetMove(wrap, "set-lift", node => at.after(node));
      after = wrap;
    }
  }
}

function phoneSetFace() {
  const line = document.querySelector(".account-card > .account-line");
  if (!line || typeof state === "undefined" || !state.user || typeof makeAvatar !== "function") return;
  const name = state.user.display_name || state.user.username || "";
  const face = makeAvatar(state.user.id, name, "avatar set-me");
  const old = line.querySelector(":scope > .set-me");
  if (old) old.replaceWith(face);
  else line.prepend(face);
}

function syncPhoneSettingsPage() {
  if (!studioSlim()) {
    phoneSetRestore();
    return;
  }
  if (!phoneSetPage.moved.length) {
    for (const page of document.querySelectorAll(".settings-layer .settings-page")) phoneSetLift(page);
  }
  phoneSetFace();
  phoneSetServerHead();
}

function wirePhoneSettingsPage() {
  const layers = document.querySelectorAll(".settings-layer");
  const watch = new MutationObserver(() => {
    if (document.querySelector(".settings-layer:not(.hidden)")) syncPhoneSettingsPage();
  });
  for (const layer of layers) watch.observe(layer, { attributes: true, attributeFilter: ["class"] });
  const hint = document.getElementById("overview-icon-hint");
  if (hint) {
    hint.addEventListener("click", () => {
      const icon = document.getElementById("overview-icon");
      if (studioSlim() && icon && !icon.disabled) icon.click();
    });
  }
}

wirePhoneSettingsPage();
