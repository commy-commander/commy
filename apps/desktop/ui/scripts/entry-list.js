const ENTRY_STATES = {
  off: "Not signed in",
  synced: "Up to date",
  waiting: "Waiting for one of your devices",
  own_identity: "This device keeps its own identity",
  unsupported: "Not available on your account server",
  error: "Could not sync",
};

let entryAsks = [];

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

function showEntryAsk() {
  const ask = entryAsks[0];
  if (!ask) {
    hideModals();
    return;
  }
  $("entry-key-device").textContent = ask.device_name || "your other device";
  $("entry-key-code").textContent = ask.code || "";
  $("entry-key-code-row").classList.toggle("hidden", !ask.code);
  showModal("modal-entry-key");
}

async function answerEntryAsk(allow) {
  const ask = entryAsks[0];
  if (!ask) return hideModals();
  const answered = entryAsksFromSameDevice(entryAsks, ask);
  entryAsks = withoutEntryAsks(entryAsks, answered);
  showEntryAsk();
  const replies = await Promise.allSettled(answered.map(a => invoke("answer_entry_key_request", { id: a.id, allow })));
  if (replies.every(r => r.status === "rejected")) toast(String(replies[0].reason));
  else if (allow) toast(`${ask.device_name} can now use your servers`);
}

$("entry-key-allow").onclick = () => answerEntryAsk(true);
$("entry-key-deny").onclick = () => answerEntryAsk(false);

if (IS_TAURI) {
  window.__TAURI__.event.listen("entry-list-changed", e => renderEntryStatus(e.payload));
  window.__TAURI__.event.listen("entry-key-request", e => {
    entryAsks = queueEntryAsk(entryAsks, e.payload);
    showEntryAsk();
  });
  window.__TAURI__.event.listen("entry-key-request-closed", e => {
    const showing = entryAsks[0];
    entryAsks = withoutEntryAskId(entryAsks, e.payload);
    if (showing && showing.id === e.payload) showEntryAsk();
  });
  refreshEntryStatus();
  document.addEventListener("visibilitychange", async () => {
    if (document.visibilityState !== "visible") return;
    const status = await invoke("entry_list_status").catch(() => null);
    if (status && status.state === "error") renderEntryStatus(await invoke("entry_list_sync").catch(() => status));
  });
}
