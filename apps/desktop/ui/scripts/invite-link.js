// Invite links: https://<server>/i/<CODE> and the commy://i/<host>/<CODE>
// deep link. The code is resolved on the server named in the link, and a
// server other than this install's account server gets a warning first.

const INVITE_LINK_RE = /\b(?:https?:\/\/[a-z0-9.\-:[\]]+\/i\/|commy:\/\/i\/[^\s/]+\/)[a-hj-np-z2-9]{8}\b\/?/i;

function findInviteLink(text) {
  const m = INVITE_LINK_RE.exec(text || "");
  return m ? m[0] : null;
}

function looksLikeInviteLink(text) {
  const link = findInviteLink(text);
  return Boolean(link) && link === String(text).trim();
}

let inviteLinkBusy = false;
let pendingInvite = null;

async function openInviteLink(link) {
  if (inviteLinkBusy) return;
  inviteLinkBusy = true;
  try {
    const invite = await invoke("resolve_invite_link", { link });
    pendingInvite = invite;
    $("invite-link-name").textContent = invite.server_name;
    const expires = new Date(invite.expires_at);
    $("invite-link-host").textContent = `Invite from ${invite.host}` +
      (isNaN(expires) ? "" : `, expires ${expires.toLocaleDateString(undefined, { month: "long", day: "numeric" })}`);
    const warning = $("invite-link-warning");
    warning.classList.toggle("hidden", invite.trusted);
    if (!invite.trusted) {
      warning.textContent = `This invite comes from ${invite.host}, not your account server (${invite.account_server_host}). ` +
        "Anyone can run a Commy server, so only join if you trust the person who sent this link. " +
        "Your account stays on your own server either way.";
    }
    showModal("modal-invite-link");
  } catch (e) {
    toast(String(e));
  } finally {
    inviteLinkBusy = false;
  }
}

async function joinPendingInvite() {
  const invite = pendingInvite;
  if (!invite) return;
  const button = $("invite-link-join");
  button.disabled = true;
  try {
    const summary = await invoke("join_server", { ticket: invite.ticket });
    pendingInvite = null;
    hideModals();
    rolePromptPending.add(summary.id);
    await refreshServers();
    await selectServer(summary.id);
    toast("Joined server, syncing from peers");
  } catch (e) {
    toast(String(e));
  } finally {
    button.disabled = false;
  }
}

// Chat messages containing an invite link get the same Join card as raw tickets.
function inviteLinkCard(link) {
  const card = el("div", "invite-card");
  card.appendChild(el("div", "invite-card-title", "You've been invited to join a server"));
  const join = el("button", "btn-primary btn-small", "Join");
  join.onclick = () => openInviteLink(link);
  card.appendChild(join);
  return card;
}

async function copyInviteLink() {
  if (!state.currentServer) return;
  const button = $("invite-link-copy");
  button.disabled = true;
  try {
    const url = await invoke("create_invite_link", { serverId: state.currentServer.id });
    $("invite-text").value = url;
    await navigator.clipboard.writeText(url);
    toast("Invite link copied");
  } catch (e) {
    toast(String(e));
  } finally {
    button.disabled = false;
  }
}

// A link the app was launched with waits until first run setup is done.
async function takePendingInviteLink() {
  if (!IS_TAURI) return;
  if (!$("modal-username").classList.contains("hidden") && !$("modal-backdrop").classList.contains("hidden")) {
    setTimeout(takePendingInviteLink, 1000);
    return;
  }
  const link = await invoke("take_pending_invite_link").catch(() => null);
  if (link) openInviteLink(link);
}

$("invite-link-join").onclick = joinPendingInvite;
$("invite-link-cancel").onclick = () => {
  pendingInvite = null;
  hideModals();
};
$("invite-link-copy").onclick = copyInviteLink;

if (IS_TAURI) {
  window.__TAURI__.event.listen("invite-link", () => takePendingInviteLink());
  setTimeout(takePendingInviteLink, 1500);
}
