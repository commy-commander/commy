const INVITE_LINK_RE = /\b(?:https?:\/\/[a-z0-9.\-:[\]]+\/i\/|commy:\/\/i\/[^\s/]+\/)[a-hj-np-z2-9]{8}(?:#[A-Za-z0-9_-]{43}(?![A-Za-z0-9_-])|\b\/?)/i;

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
    $("invite-link-icon").textContent = initials(invite.server_name);
    const expires = invite.expires_at ? new Date(invite.expires_at) : null;
    $("invite-link-host").textContent = `Invite from ${invite.host}` +
      (!expires ? ", never expires" : isNaN(expires) ? "" : `, expires ${expires.toLocaleDateString(undefined, { month: "long", day: "numeric" })}`);
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
    await enterJoinedServer(summary);
  } catch (e) {
    toast(String(e));
  } finally {
    button.disabled = false;
  }
}

function inviteLinkCard(link) {
  const card = el("div", "invite-card");
  const mark = el("div", "invite-card-mark");
  mark.appendChild(svgIcon("serverAdd", 18));
  card.appendChild(mark);
  const text = el("div", "invite-card-text");
  text.appendChild(el("div", "invite-card-title", "You've been invited to join a server"));
  const host = (/^commy:\/\/i\/([^/]+)\//i.exec(link) || [])[1] || (/^https?:\/\/([^/]+)/i.exec(link) || [])[1] || "";
  text.appendChild(el("div", "invite-card-sub", host ? `Invite link from ${host}` : "Invite link"));
  card.appendChild(text);
  const join = el("button", "btn-primary btn-small", "Join");
  join.onclick = () => openInviteLink(link);
  card.appendChild(join);
  return card;
}

const INVITE_EXPIRY_NAMES = { "1h": "1 hour", "1d": "1 day", "7d": "7 days", "30d": "30 days" };
let inviteExpiry = "7d";
let inviteLinkShown = false;

function inviteExpiryText(expiresAt) {
  if (!expiresAt) return "This link never expires.";
  const at = new Date(expiresAt);
  if (isNaN(at)) return "";
  const soon = at - Date.now() < 36 * 3600e3;
  const when = soon
    ? at.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })
    : at.toLocaleDateString(undefined, { month: "long", day: "numeric" });
  return soon && at.getDate() !== new Date().getDate()
    ? `This link expires tomorrow at ${when}.`
    : `This link expires ${soon ? "at" : "on"} ${when}.`;
}

function renderInviteExpiry() {
  const isPublic = Boolean(state.currentServer?.public);
  if (inviteExpiry === "never" && !isPublic) inviteExpiry = "7d";
  $("invite-expiry-never").classList.toggle("hidden", !isPublic);
  for (const btn of $("invite-expiry").querySelectorAll("button")) {
    const on = btn.dataset.expires === inviteExpiry;
    btn.classList.toggle("active", on);
    btn.setAttribute("aria-checked", String(on));
  }
  if (!inviteLinkShown) {
    $("invite-expiry-note").textContent = inviteExpiry === "never"
      ? "Copy Invite Link for a link that never expires."
      : `Copy Invite Link for a link that lasts ${INVITE_EXPIRY_NAMES[inviteExpiry]}.`;
  }
}

function resetInviteDialog() {
  inviteLinkShown = false;
  renderInviteExpiry();
}

async function makeInviteLink() {
  const invite = await invoke("create_invite_link", { serverId: state.currentServer.id, expires: inviteExpiry });
  $("invite-text").value = invite.url;
  $("invite-expiry-note").textContent = inviteExpiryText(invite.expires_at);
  inviteLinkShown = true;
  return invite.url;
}

async function copyInviteLink() {
  if (!state.currentServer) return;
  const button = $("invite-link-copy");
  button.disabled = true;
  try {
    const url = await makeInviteLink();
    await navigator.clipboard.writeText(url);
    toast("Invite link copied");
  } catch (e) {
    toast(String(e));
  } finally {
    button.disabled = false;
  }
}

for (const btn of $("invite-expiry").querySelectorAll("button")) {
  btn.onclick = async () => {
    if (btn.dataset.expires === inviteExpiry) return;
    inviteExpiry = btn.dataset.expires;
    renderInviteExpiry();
    if (inviteLinkShown && state.currentServer) {
      try {
        await makeInviteLink();
      } catch (e) {
        toast(String(e));
      }
    }
  };
}

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
