const ENTRY_STATES = {
  off: "Not signed in",
  synced: "Up to date",
  waiting: "Waiting for one of your devices",
  own_identity: "This device keeps its own identity",
  unsupported: "Not available on your account server",
  error: "Could not sync",
};

function renderEntryStatus(status) {
  if (!status || !$("entry-state")) return;
  const title = ENTRY_STATES[status.state] || ENTRY_STATES.error;
  $("entry-state").textContent = status.state === "synced"
    ? `Up to date, ${status.servers} ${status.servers === 1 ? "server" : "servers"}`
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
  $("entry-held-title").textContent = `Your server list asks this device to leave ${held.length} servers at once`;
  $("entry-held-names").textContent = `${held.filter(Boolean).join(", ")}. If you did not leave them on another device, keep them.`;
  if (heldShown !== held.length) toast("Your server list asks this device to leave several servers. See Settings, Commy Account.");
  heldShown = held.length;
}

async function answerHeldLeaves(leave) {
  const buttons = [$("entry-held-keep"), $("entry-held-leave")];
  buttons.forEach(b => { b.disabled = true; });
  try {
    renderEntryStatus(await invoke("entry_list_answer_held", { leave }));
    toast(leave ? "Left those servers on this device" : "Kept those servers");
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
  return at && !isNaN(at) ? at.toLocaleString() : "Unknown time";
}

function earlierRow(view) {
  const row = el("li", "entry-earlier-row");
  const text = el("div");
  const count = `${view.servers} ${view.servers === 1 ? "server" : "servers"}`;
  text.append(el("div", "setting-title", `${earlierWhen(view)}, ${count}`));
  if (view.writer) text.append(el("div", "setting-sub", `Saved by ${view.writer}`));
  const back = view.brings_back.filter(Boolean);
  text.append(el("div", "setting-sub", back.length ? `Brings back: ${back.join(", ")}` : "Nothing to bring back, this device has all of these"));
  row.append(text);
  if (back.length) {
    const button = el("button", "btn-secondary btn-small", "Bring Back");
    button.onclick = () => bringBack(view, button);
    row.append(button);
  }
  return row;
}

async function bringBack(view, button) {
  button.disabled = true;
  $("entry-earlier-status").textContent = "Rejoining…";
  try {
    const result = await invoke("entry_list_bring_back", { version: view.version });
    const joined = result.joined.length;
    $("entry-earlier-status").textContent = result.failed.length
      ? `Rejoined ${joined}. Could not rejoin: ${result.failed.join(", ")}. Ask for a new invite to those.`
      : `Rejoined ${joined} ${joined === 1 ? "server" : "servers"}.`;
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
    $("entry-earlier-status").textContent = "Opening earlier copies…";
    showModal("modal-entry-earlier");
  }
  try {
    const views = await invoke("entry_list_earlier");
    list.replaceChildren(...views.map(earlierRow));
    if (fresh) $("entry-earlier-status").textContent = views.length ? "" : "No earlier copies yet. They are kept from now on, each time your list changes.";
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
    toast("Recovery code accepted");
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
    toast("Started a new server list");
  } catch (e) {
    toast(String(e));
  } finally {
    button.disabled = false;
  }
};

if (IS_TAURI) {
  window.__TAURI__.event.listen("entry-list-changed", e => renderEntryStatus(e.payload));
  window.__TAURI__.event.listen("web-sync-stalled", e => {
    const reason = (e.payload && e.payload.reason) || "Your servers are taking longer than usual to load.";
    console.warn(`Commy: ${reason}`);
    toast(`${reason} Reload the page. If servers are still missing, open Settings, Commy Account and choose Bring back earlier servers.`);
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
  $("acct-moved-title").textContent = to ? `Your account moved to ${to.replace(/^https:\/\//, "")}` : "";
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
  $("account-move-go").textContent = "Move Account";
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
  showMoveStatus(`Copying your account to ${shownHost(movingTo)}…`);
  try {
    await invoke("start_account_move", { to: movingTo });
    movingTo = null;
    hideModals();
    await afterAccountMove();
  } catch (e) {
    showMoveStatus(moveProblem(e), true);
    button.textContent = "Try Again";
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
    if (e.payload && e.payload.moved_to) toast("Your account moved to another account server. See Settings, Commy Account.");
  });
  invoke("account_moved_to").then(renderAccountMoved).catch(() => {});
}
