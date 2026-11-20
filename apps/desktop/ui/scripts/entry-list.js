const ENTRY_STATES = {
  get off() { return t("ui.not-signed-in"); },
  get synced() { return t("ui.up-to-date"); },
  get waiting() { return t("ui.waiting-for-one-of-your-devices"); },
  get own_identity() { return t("ui.this-device-keeps-its-own-identity"); },
  get unsupported() { return t("ui.not-available-on-your-account-server"); },
  get error() { return t("ui.could-not-sync"); },
};

function renderEntryStatus(status) {
  if (!status || !$("entry-state")) return;
  const title = ENTRY_STATES[status.state] || ENTRY_STATES.error;
  $("entry-state").textContent = status.state === "synced"
    ? t(status.servers === 1 ? "ui.up-to-date-servers.one" : "ui.up-to-date-servers.many", { count: status.servers })
    : title;
  $("entry-detail").textContent = status.detail || "";
  $("entry-show-code").classList.toggle("hidden", !status.has_key);
  $("entry-enter").classList.toggle("hidden", status.has_key || status.state === "off" || status.state === "unsupported");
  $("entry-start-over-row").classList.toggle("hidden", status.has_key || status.state !== "waiting");
  renderHeldLeaves(status.held || []);
}

let heldShown = 0;

function renderHeldLeaves(held) {
  $("entry-held").classList.toggle("hidden", !held.length);
  if (!held.length) {
    heldShown = 0;
    return;
  }
  $("entry-held-title").textContent = t("ui.held-leave-title", { count: held.length });
  $("entry-held-names").textContent = t("ui.held-leave-names", { names: held.filter(Boolean).join(", ") });
  if (heldShown !== held.length) toast(t("ui.held-leave-toast"));
  heldShown = held.length;
}

async function answerHeldLeaves(leave) {
  const buttons = [$("entry-held-keep"), $("entry-held-leave")];
  buttons.forEach(b => { b.disabled = true; });
  try {
    renderEntryStatus(await invoke("entry_list_answer_held", { leave }));
    toast(t(leave ? "ui.held-left" : "ui.held-kept"));
  } catch (e) {
    toast(String(e));
  } finally {
    buttons.forEach(b => { b.disabled = false; });
  }
}

$("entry-held-keep").onclick = () => answerHeldLeaves(false);
$("entry-held-leave").onclick = () => answerHeldLeaves(true);

function earlierWhen(view) {
  const at = view.saved_at ? new Date(view.saved_at) : null;
  return at && !isNaN(at) ? at.toLocaleString(dateLocale()) : t("ui.unknown-time");
}

function earlierRow(view) {
  const row = el("li", "entry-earlier-row");
  const text = el("div");
  const count = t(view.servers === 1 ? "ui.earlier-servers.one" : "ui.earlier-servers.many", { count: view.servers });
  text.append(el("div", "setting-title", `${earlierWhen(view)}, ${count}`));
  if (view.writer) text.append(el("div", "setting-sub", t("ui.saved-by", { name: view.writer })));
  const back = view.brings_back.filter(Boolean);
  text.append(el("div", "setting-sub", back.length ? t("ui.brings-back", { names: back.join(", ") }) : t("ui.nothing-to-bring-back")));
  row.append(text);
  if (back.length) {
    const button = el("button", "btn-secondary btn-small", t("ui.bring-back"));
    button.onclick = () => bringBack(view, button);
    row.append(button);
  }
  return row;
}

async function bringBack(view, button) {
  button.disabled = true;
  $("entry-earlier-status").textContent = t("ui.rejoining");
  try {
    const result = await invoke("entry_list_bring_back", { version: view.version });
    const joined = result.joined.length;
    $("entry-earlier-status").textContent = result.failed.length
      ? t("ui.rejoined-some", { count: joined, names: result.failed.join(", ") })
      : t(joined === 1 ? "ui.rejoined.one" : "ui.rejoined.many", { count: joined });
    await openEarlierLists(false);
  } catch (e) {
    $("entry-earlier-status").textContent = String(e);
  } finally {
    button.disabled = false;
  }
}

async function openEarlierLists(fresh = true) {
  const list = $("entry-earlier-list");
  if (fresh) {
    list.replaceChildren();
    $("entry-earlier-status").textContent = t("ui.opening-earlier-copies");
    showModal("modal-entry-earlier");
  }
  try {
    const views = await invoke("entry_list_earlier");
    list.replaceChildren(...views.map(earlierRow));
    if (fresh) $("entry-earlier-status").textContent = views.length ? "" : t("ui.no-earlier-copies-yet");
  } catch (e) {
    $("entry-earlier-status").textContent = String(e);
  }
}

$("entry-earlier").onclick = () => openEarlierLists();
$("entry-earlier-close").onclick = hideModals;

async function refreshEntryStatus() {
  if (!IS_TAURI) return;
  renderEntryStatus(await invoke("entry_list_status").catch(() => null));
}

$("entry-sync").onclick = async () => {
  const button = $("entry-sync");
  button.disabled = true;
  try {
    renderEntryStatus(await invoke("entry_list_sync"));
  } catch (e) {
    toast(String(e));
    refreshEntryStatus();
  } finally {
    button.disabled = false;
  }
};

$("entry-show-code").onclick = async () => {
  try {
    const code = await invoke("entry_list_recovery_code");
    const input = $("entry-code");
    input.value = code;
    input.classList.remove("hidden");
    $("entry-code-note").classList.remove("hidden");
    $("entry-show-code").classList.add("hidden");
    input.select();
  } catch (e) {
    toast(String(e));
  }
};

$("entry-code-use").onclick = async () => {
  const button = $("entry-code-use");
  button.disabled = true;
  try {
    renderEntryStatus(await invoke("entry_list_use_recovery_code", { code: $("entry-code-input").value }));
    $("entry-code-input").value = "";
    toast(t("ui.recovery-code-accepted"));
  } catch (e) {
    toast(String(e));
  } finally {
    button.disabled = false;
  }
};

$("entry-start-over").onclick = () => showModal("modal-entry-start-over");
$("entry-start-over-cancel").onclick = hideModals;
$("entry-start-over-go").onclick = async () => {
  const button = $("entry-start-over-go");
  button.disabled = true;
  try {
    renderEntryStatus(await invoke("entry_list_start_over"));
    hideModals();
    toast(t("ui.started-a-new-server-list"));
  } catch (e) {
    toast(String(e));
  } finally {
    button.disabled = false;
  }
};

if (IS_TAURI) {
  window.__TAURI__.event.listen("entry-list-changed", e => renderEntryStatus(e.payload));
  window.__TAURI__.event.listen("web-sync-stalled", e => {
    const reason = (e.payload && e.payload.reason) || t("ui.servers-taking-longer");
    console.warn(`Commy: ${reason}`);
    toast(`${reason} ${t("ui.servers-stalled-reload")}`);
  });
  window.__TAURI__.event.listen("web-server-look-back", e => {
    const { server, seq } = e.payload || {};
    console.info(`Commy: no recent copy of ${server} in the last entries, reading all ${seq}`);
  });
  window.__TAURI__.event.listen("web-home-lookup", e => {
    const { home, status, sync } = e.payload || {};
    const line = `Commy: looked up ${home}: ${status ?? "no answer"}, sync ${sync ?? "unknown"}`;
    if (sync) console.info(line);
    else console.warn(line);
  });
  refreshEntryStatus();
  document.addEventListener("visibilitychange", async () => {
    if (document.visibilityState !== "visible") return;
    const status = await invoke("entry_list_status").catch(() => null);
    if (status && status.state === "error") renderEntryStatus(await invoke("entry_list_sync").catch(() => status));
  });
}

function renderAccountMoved(view) {
  const to = view && view.moved_to;
  $("acct-moved-row").classList.toggle("hidden", !to);
  $("acct-moved-title").textContent = to ? t("ui.your-account-moved-to", { value: to.replace(/^https:\/\//, "") }) : "";
}

function moveProblem(e) {
  const message = String(e);
  return isServerProblem(message) ? t(message) : message;
}

async function afterAccountMove() {
  account = await invoke("account_status").catch(() => account);
  startOverSignedOut();
}

function fillList(list, items, tag = "li") {
  list.replaceChildren(...items.map(text => el(tag, null, text)));
}

let movingTo = null;

function showMoveStatus(text, failed = false) {
  const status = $("account-move-status");
  status.textContent = text;
  status.classList.toggle("error", failed);
}

function openMovePlan(typed) {
  const plan = movePlan(typed);
  movingTo = typed;
  $("account-move-title").textContent = plan.title;
  $("account-move-comes").textContent = plan.comes;
  fillList($("account-move-steps"), plan.steps);
  $("account-move-others").textContent = plan.others;
  $("account-move-note").textContent = plan.note;
  showMoveStatus("");
  $("account-move-go").textContent = t("ui.move-account");
  $("account-move-go").disabled = false;
  showModal("modal-account-move");
}

$("acct-move-go").onclick = () => {
  const typed = $("acct-move-input").value.trim();
  if (typed) openMovePlan(typed);
};

$("account-move-cancel").onclick = hideModals;

$("account-move-go").onclick = async () => {
  if (!movingTo) return;
  const button = $("account-move-go");
  button.disabled = true;
  showMoveStatus(t("ui.copying-your-account-to", { value: shownHost(movingTo) }));
  try {
    await invoke("start_account_move", { to: movingTo });
    movingTo = null;
    hideModals();
    await afterAccountMove();
  } catch (e) {
    showMoveStatus(moveProblem(e), true);
    button.textContent = t("ui.try-again");
  } finally {
    button.disabled = false;
  }
};

function showMovedIn(moved) {
  const welcome = movedInWelcome(moved);
  $("moved-in-title").textContent = welcome.title;
  $("moved-in-lead").textContent = welcome.lead;
  fillList($("moved-in-points"), welcome.points);
  showModal("modal-moved-in");
}

async function showMovedInIfDue() {
  const moved = await invoke("take_account_moved_in").catch(() => null);
  if (!moved) return false;
  showMovedIn(moved);
  return true;
}

$("moved-in-done").onclick = hideModals;

async function showMoveFinishNote() {
  const to = await invoke("account_move_pending").catch(() => null);
  const note = $("signin-move-note");
  note.classList.toggle("hidden", !to);
  note.textContent = to ? t("signin.move.finish", { host: shownHost(to) }) : "";
}

$("acct-moved-follow").onclick = async () => {
  const button = $("acct-moved-follow");
  button.disabled = true;
  try {
    await invoke("follow_account_move");
    await afterAccountMove();
  } catch (e) {
    toast(moveProblem(e));
  } finally {
    button.disabled = false;
  }
};

if (IS_TAURI) {
  window.__TAURI__.event.listen("account-moved", e => {
    renderAccountMoved(e.payload);
    if (e.payload && e.payload.moved_to) toast(t("ui.your-account-moved-to-another-account"));
  });
  invoke("account_moved_to").then(renderAccountMoved).catch(() => {});
}
