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
}

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

$("acct-move-go").onclick = async () => {
  const typed = $("acct-move-input").value.trim();
  if (!typed) return;
  const host = typed.replace(/^https:\/\//, "").replace(/\/+$/, "");
  if (!(await confirm(`Move your account to ${host}? This device signs out here, then you sign in or create an account on ${host}.`))) return;
  const button = $("acct-move-go");
  button.disabled = true;
  try {
    await invoke("start_account_move", { to: typed });
    await afterAccountMove();
  } catch (e) {
    toast(moveProblem(e));
  } finally {
    button.disabled = false;
  }
};

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
