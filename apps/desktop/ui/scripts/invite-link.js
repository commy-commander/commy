let inviteLinkBusy = false;
let pendingInvite = null;

async function openInviteLink(link) {
  if (inviteLinkBusy) return;
  inviteLinkBusy = true;
  try {
    const invite = await invoke("resolve_invite_link", { link });
    pendingInvite = invite;
    $("invite-link-name").textContent = serverShownName(invite.server_name);
    $("invite-link-icon").textContent = initials(serverShownName(invite.server_name));
    const hero = $("invite-link-hero");
    hero.style.backgroundImage = invite.background ? `url("${invite.background}")` : "";
    $("modal-invite-link").classList.toggle("has-hero", Boolean(invite.background));
    const expires = invite.expires_at ? new Date(invite.expires_at) : null;
    $("invite-link-host").textContent = t("ui.invite-from", { host: invite.host }) +
      (!expires ? ", never expires" : isNaN(expires) ? "" : `, expires ${expires.toLocaleDateString(dateLocale(), { month: "long", day: "numeric" })}`);
    const warning = $("invite-link-warning");
    const warningText = inviteWarningText(invite);
    warning.classList.toggle("hidden", !warningText);
    if (warningText) warning.textContent = warningText;
    const learn = $("invite-link-learn");
    learn.classList.toggle("hidden", !warningText);
    $("invite-link-join").textContent = inviteAsksTrust(invite) ? TRUST_AND_JOIN : t("ui.join-server");
    showModal("modal-invite-link");
  } catch (e) {
    toast(String(e));
  } finally {
    inviteLinkBusy = false;
  }
}

async function trustInviteHost(invite) {
  if (inviteAsksTrust(invite)) await invoke("trust_sync_host", { host: invite.community_host.origin });
}

async function joinPendingInvite() {
  const invite = pendingInvite;
  if (!invite) return;
  const button = $("invite-link-join");
  button.disabled = true;
  try {
    await trustInviteHost(invite);
    const summary = await invoke("join_server", { ticket: invite.ticket, link: invite.link });
    pendingInvite = null;
    hideModals();
    await enterJoinedServer(summary);
  } catch (e) {
    toast(String(e));
  } finally {
    button.disabled = false;
  }
}

function inviteCardMark() {
  const mark = el("div", "invite-card-mark");
  const studio = document.documentElement.dataset.look === "studio" && typeof studioIconNode === "function";
  mark.appendChild(studio ? studioIconNode("user-plus", 21, false) : svgIcon("serverAdd", 18));
  return mark;
}

function inviteLinkCard(link) {
  const card = el("div", "invite-card");
  card.appendChild(inviteCardMark());
  const text = el("div", "invite-card-text");
  text.appendChild(el("div", "invite-card-title", t("ui.you-ve-been-invited-to-join-a-server")));
  const host = (/^commy:\/\/i\/([^/]+)\//i.exec(link) || [])[1] || (/^https?:\/\/([^/]+)/i.exec(link) || [])[1] || "";
  text.appendChild(el("div", "invite-card-sub", host ? t("ui.invite-link-from", { host }) : t("ui.invite-link")));
  card.appendChild(text);
  const join = el("button", "btn-primary btn-small", t("ui.join"));
  join.onclick = () => openInviteLink(link);
  card.appendChild(join);
  return card;
}

const INVITE_EXPIRY_NAMES = { "1h": "1 hour", "1d": "1 day", "7d": "7 days", "30d": "30 days" };
let inviteExpiry = "7d";
let inviteLinkShown = false;

function inviteExpiryText(expiresAt) {
  if (!expiresAt) return t("ui.this-link-never-expires");
  const at = new Date(expiresAt);
  if (isNaN(at)) return "";
  const soon = at - Date.now() < 36 * 3600e3;
  const when = soon
    ? at.toLocaleTimeString(dateLocale(), { hour: "numeric", minute: "2-digit" })
    : at.toLocaleDateString(dateLocale(), { month: "long", day: "numeric" });
  return soon && at.getDate() !== new Date().getDate()
    ? t("ui.this-link-expires-tomorrow-at", { when })
    : t(soon ? "ui.this-link-expires-at" : "ui.this-link-expires-on", { when });
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
  const key = inviteKeyLabels().dialog;
  $("invite-link-copy").textContent = key;
  if (!inviteLinkShown) {
    $("invite-expiry-note").textContent = inviteExpiry === "never"
      ? t("ui.invite-key-never-expires", { key })
      : t("ui.invite-key-lasts", { key, value: INVITE_EXPIRY_NAMES[inviteExpiry] });
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
    if (inviteShareHere()) await shareFullInviteLink(url, state.currentServer?.name);
    else {
      await navigator.clipboard.writeText(url);
      toast(t("ui.invite-link-copied"));
    }
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

const ONBOARDING_MODALS = ["modal-signin", "modal-claim", "modal-username"];

function onboardingDone() {
  if (!state.user) return false;
  if ($("modal-backdrop").classList.contains("hidden")) return true;
  return ONBOARDING_MODALS.every(id => $(id).classList.contains("hidden"));
}

async function takePendingInviteLink() {
  if (!IS_TAURI || !onboardingDone()) return false;
  const link = await invoke("take_pending_invite_link").catch(() => null);
  if (!link) return false;
  await openInviteLink(link);
  return true;
}

$("invite-link-join").onclick = joinPendingInvite;
$("invite-link-cancel").onclick = () => {
  pendingInvite = null;
  hideModals();
};
$("invite-link-copy").onclick = copyInviteLink;

if (IS_TAURI) {
  window.__TAURI__.event.listen("invite-link", () => takePendingInviteLink());
}
