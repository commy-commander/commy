const STUDIO_TABS = [
  { id: "home", label: "Home", icon: "house" },
  { id: "messages", label: "Messages", icon: "message-circle" },
  { id: "live", label: "Live", icon: "radio" },
  { id: "you", label: "You", icon: "user" },
];
const STUDIO_TITLES = { messages: "Messages", live: "Live", you: "You" };
const STUDIO_LIVE_REFRESH_MS = 15000;
const STUDIO_HOME_WORDS = { "Server Rules": "Rules", "Text Channels": "Text channels", "Voice Channels": "Voice channels" };

const studioPhone = {
  tab: "home",
  lastServer: null,
  armed: false,
  fuse: 0,
  centres: [],
  centre: 0,
  swell: HORIZON.swell,
  ping: 0,
  springs: {},
  frame: 0,
  frameAt: 0,
  followUntil: 0,
  rooms: [],
  servers: [],
  roomSeen: "",
  roomQueued: 0,
  soundsWereOpen: false,
  trayWasOpen: false,
  trayStep: false,
  inviteStep: false,
  inviteBusy: false,
  trayHome: null,
  talkHeld: false,
  roomsAt: 0,
  roomsBusy: false,
  liveTimer: 0,
  badge: 0,
  calm: false,
  rowOpen: null,
  swipedAt: 0,
  railTimer: 0,
  bubblesSeen: "",
  streamShown: false,
  toolsTimer: 0,
  searchHome: null,
  waitsOpen: false,
  groupFaces: new Map(),
  searchPick: false,
};

function studioSlim() {
  return studioLook() && document.documentElement.dataset.fit === "slim";
}

function studioNode(tag, className, html) {
  const node = el(tag, className);
  if (html) {
    node.innerHTML = html;
    StudioIcons.paint(node);
  }
  return node;
}

function horizonMarkup() {
  const extras = { messages: '<span class="badge hidden"></span>', live: '<span class="ld hidden"></span>' };
  const tabs = STUDIO_TABS.map(tab => `<button type="button" class="tab" role="tab" data-tab="${tab.id}" aria-selected="false"><span class="ic"><i data-i="${tab.icon}"></i>${extras[tab.id] || ""}</span><span class="lb">${tab.label}</span></button>`).join("");
  return `<i class="hzn-ground"></i><span class="hzn-sun"></span><span class="hzn-sun warm"></span>
<svg class="hzn-line" aria-hidden="true"><defs>
<linearGradient id="hzn-g" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="390" y2="0"><stop offset="0" style="stop-color:var(--tint-1)" stop-opacity="0"/><stop offset=".14" style="stop-color:var(--tint-1)"/><stop offset=".7" style="stop-color:var(--accent-lit)"/><stop offset="1" style="stop-color:var(--tint-1)" stop-opacity="0"/></linearGradient>
<linearGradient id="hzn-hot" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="124" y2="0"><stop offset="0" style="stop-color:var(--white)" stop-opacity="0"/><stop offset=".5" style="stop-color:var(--white)" stop-opacity=".85"/><stop offset="1" style="stop-color:var(--white)" stop-opacity="0"/></linearGradient>
</defs><path class="glow"/><path class="core" stroke="url(#hzn-g)"/><path class="hot" stroke="url(#hzn-hot)"/></svg>
<i class="hzn-fuse"></i>
<div class="hzn-tabs" role="tablist" aria-label="Places">${tabs}</div>
<i class="hzn-tick"></i>
<div class="hzn-voice">
<div class="ks"><button type="button" class="key sq tap mic" aria-label="Mute"><i data-i="mic"></i><i class="led"></i></button><button type="button" class="key sq tap danger hang" aria-label="Hang up"><i data-i="phone-off"></i></button></div>
<button type="button" class="open" aria-label="Voice controls"><i data-i="bars"></i><span class="open-name"></span></button>
<div class="ask" role="status">Tap again to leave</div>
</div>
<div class="hzn-room">${ROOM_KEYS.map(roomKeyMarkup).join("")}<div class="ask" role="status">Tap again to leave</div></div>`;
}

function roomKeyMarkup(key) {
  const kind = key.id === "mic" ? "lg cream mic" : key.id === "leave" ? "tap danger hang" : "tap";
  const lamp = ROOM_LAMPS.includes(key.id) ? '<i class="led"></i>' : "";
  return `<div class="k${key.id === "mic" ? " big" : ""}"><button type="button" class="key sq ${kind}" data-room-key="${key.id}" aria-label="${key.label}"><i data-i="${key.icon}"></i>${lamp}</button><span class="lb">${key.label}</span></div>`;
}

function voiceSheetMarkup() {
  return `<h3>Voice</h3>
<div class="arow conn" id="pt-voice-conn">${studioDockSignalMarkup()}<span class="l"><b></b><small></small></span></div>
<button type="button" class="arow" data-caret="mute-caret"><i data-i="mic"></i><span class="l">Microphone</span><small id="pt-voice-mic"></small></button>
<button type="button" class="arow" data-caret="deafen-caret"><i data-i="headphones"></i><span class="l">Speaker</span><small id="pt-voice-out"></small></button>
<button type="button" class="arow" data-caret="camera-caret"><i data-i="video"></i><span class="l">Camera</span><small id="pt-voice-cam"></small></button>
<button type="button" class="arow" id="pt-voice-share" data-press="share-btn"><i data-i="monitor-up"></i><span class="l"></span></button>
<button type="button" class="arow" id="pt-voice-ink" role="switch" aria-checked="false"><i data-i="pencil"></i><span class="l">Let viewers draw</span><i class="tog"></i></button>
<button type="button" class="arow" id="pt-voice-act" data-press="vp-activities"><i data-i="gamepad-2"></i><span class="l"></span></button>`;
}

function roomMarkup() {
  return `<header class="pt-room-head">
<button type="button" class="pt-room-down" id="pt-room-down" aria-label="Back, and stay in the room"><i data-i="chevron-down"></i></button>
<div class="t"><b><span id="pt-room-name"></span><span class="live hidden" id="pt-room-live">Live</span></b><small id="pt-room-sub"></small></div>
<button type="button" class="sig" id="pt-room-sig" aria-haspopup="dialog" aria-expanded="false">${studioDockSignalMarkup()}<span id="pt-room-sig-word"></span></button>
</header>
<div class="pt-room-body"><div class="pt-room-streams" id="pt-room-streams"></div><div class="pt-room-people" id="pt-room-people"></div><div class="pt-aud hidden" id="pt-room-aud"><div class="pt-aud-label" id="pt-room-aud-label"></div><div class="pt-aud-chips" id="pt-room-aud-chips"></div></div></div>`;
}

function watchMarkup() {
  const key = id => {
    const spec = ROOM_KEYS.find(found => found.id === id);
    return `<button type="button" class="key sq ${id === "mic" ? "lg cream mic" : "tap"}" data-room-key="${id}" aria-label="${spec.label}"><i data-i="${spec.icon}"></i>${id === "deafen" ? '<i class="led"></i>' : ""}</button>`;
  };
  return `<div class="winfo">
<button type="button" class="key sq tap pt-watch-down" id="pt-watch-down" aria-label="Back to the room"><i data-i="chevron-down"></i></button>
<div class="wcard"><div class="t"><span class="live">Live</span><span id="pt-watch-title"></span></div><div class="s"><i data-i="volume-2"></i><span id="pt-watch-room"></span><span class="sg" id="pt-watch-sig">${studioDockSignalMarkup()}</span></div></div>
<span class="stack" id="pt-watch-people"></span>
</div>
<div class="bubbles" id="pt-watch-bubbles"></div>
<div class="ctl">${["sounds", "camera", "deafen", "mic"].map(key).join("")}</div>
<div class="wbar"><button type="button" class="key sq tap danger hang" data-room-key="leave" aria-label="Hang up"><i data-i="phone-off"></i></button><button type="button" class="wfield" id="pt-watch-field"></button></div>`;
}

function youMarkup() {
  return `<div class="acct slab">
<button type="button" class="key sq tap edit" id="pt-edit" aria-label="Edit profile"><i data-i="pencil"></i></button>
<span class="avatar" id="pt-me-avatar"></span>
<div class="t"><b id="pt-me-name"></b><button type="button" class="presence" id="pt-presence"><span class="pw" id="pt-me-status"></span><span class="pend" id="pt-me-until"></span><i data-i="chevron-down" class="pchev"></i></button></div>
</div>
<div class="set slab">
<div class="sec lbl">Appearance</div>
<div class="srow"><i data-i="sun"></i><span class="l">Theme</span>
<div class="seg" role="group" aria-label="Theme"><button type="button" class="key tap" data-theme-choice="dark" aria-pressed="false"><i class="led"></i>Night</button><button type="button" class="key tap" data-theme-choice="light" aria-pressed="false"><i class="led"></i>Day</button></div>
</div>
<button type="button" class="srow" id="pt-glass" role="switch" aria-checked="false"><i data-i="layers"></i><span class="l">Glass<small>Off is lighter on the battery</small></span><i class="tog"></i></button>
<div class="sec lbl" id="pt-notify-lbl">Notify me about</div>
<div class="picks" role="radiogroup" aria-labelledby="pt-notify-lbl">${NOTIFY_PICKS.map(pick => `<button type="button" class="srow pick" role="radio" aria-checked="false" data-notify="${pick.level}"><i data-i="${pick.icon}"></i><span class="l">${pick.label}</span><i data-i="check" class="tick"></i></button>`).join("")}</div>
<button type="button" class="srow go" id="pt-settings"><i data-i="settings"></i><span class="l">All settings</span><i data-i="chevron-right" class="chev"></i></button>
<button type="button" class="srow go hidden" id="pt-switch-account"><i data-i="users"></i><span class="l">Switch Account</span><i data-i="chevron-right" class="chev"></i></button>
<div class="about"><span class="star" id="pt-about-star" data-state="quiet" data-follow="off" role="img" aria-label="Commy"></span><span class="l" id="pt-about-words">Early build<small>Some of this will move.</small></span><button type="button" class="out" id="pt-sign-out">Sign out</button></div>
</div>`;
}

function profileMarkup() {
  const field = (label, id, max) => `<label class="fld"><span class="lbl">${label}</span><span class="in"><input type="text" id="${id}" maxlength="${max}" autocomplete="off" aria-label="${label}"></span></label>`;
  return `<header class="phd"><button type="button" class="back" id="pt-prof-back" aria-label="Back"><i data-i="chevron-left"></i></button><h2>Profile</h2><button type="button" class="key primary save" id="pt-prof-save" disabled>Save</button></header>
<div class="pedit slab"><div class="escroll" id="pt-prof-scroll">
<div class="eban"><div class="art" id="pt-prof-ban"></div><button type="button" class="key sq" id="pt-prof-ban-rm" aria-label="Remove banner"><i data-i="x"></i></button></div>
<div class="eid"><span class="eav" id="pt-prof-av"></span><div class="t"><b id="pt-prof-name"></b><small id="pt-prof-sub"></small></div></div>
<div class="epair"><button type="button" class="key" id="pt-prof-avatar"><i data-i="user"></i>Change avatar</button><button type="button" class="key" id="pt-prof-banner"><i data-i="image"></i><span id="pt-prof-banner-words">Change banner</span></button></div>
${field("Display name", "pt-prof-dn", 64)}
${field("Pronouns", "pt-prof-pro", 24)}
<div class="fld"><span class="lbl" id="pt-prof-status-lbl">Status</span><span class="in"><button type="button" class="e" id="pt-prof-emoji" aria-label="Status emoji"></button><input type="text" id="pt-prof-status" maxlength="64" autocomplete="off" aria-labelledby="pt-prof-status-lbl"></span></div>
<div class="fld" id="pt-prof-clear-fld"><span class="lbl">Clear after</span><button type="button" class="in pick" id="pt-prof-clear"><span class="v" id="pt-prof-clear-v"></span><i data-i="chevron-down"></i></button></div>
<label class="fld"><span class="lbl">About me</span><span class="in area"><textarea id="pt-prof-about" maxlength="190" rows="3" aria-label="About me"></textarea><span class="afoot"><small id="pt-prof-count"></small><button type="button" class="key sq" id="pt-prof-about-emoji" aria-label="Emoji"><i data-i="smile"></i></button></span></span></label>
<div class="fld"><span class="lbl">Server tag</span><button type="button" class="in pick" id="pt-prof-tag"><span class="v" id="pt-prof-tag-v"></span><i data-i="chevron-down"></i></button><small class="hint" id="pt-prof-tag-hint"></small></div>
<span class="lbl g" id="pt-prof-borders-lbl">Avatar border</span>
<div class="looks" id="pt-prof-borders" role="radiogroup" aria-labelledby="pt-prof-borders-lbl"></div>
<span class="lbl g" id="pt-prof-effects-lbl">Profile effect</span>
<div class="looks" id="pt-prof-effects" role="radiogroup" aria-labelledby="pt-prof-effects-lbl"></div>
</div></div>`;
}

function inviteMarkup() {
  return `<div class="iwall" id="pt-invite-wall"></div>
<section class="isheet" role="dialog" aria-labelledby="pt-invite-name">
<div class="ibody">
<div class="sico" id="pt-invite-tile" aria-hidden="true"></div>
<small>You've been invited to join a server</small>
<b id="pt-invite-name"></b>
<span class="ifrom" id="pt-invite-from"></span>
<div class="iwarn hidden" id="pt-invite-warn" role="alert"><span class="iw"><i data-i="triangle-alert"></i></span><p><b id="pt-invite-warn-lead"></b><span id="pt-invite-warn-rest"></span></p></div>
<p class="ierr hidden" id="pt-invite-error" role="alert"></p>
</div>
<div class="ikeys"><button type="button" class="key lg primary" id="pt-invite-join">Join</button><button type="button" class="key lg" id="pt-invite-later">Not now</button></div>
</section>`;
}

function buildStudioPhone() {
  const app = $("app");
  const title = studioNode("header", "pt-title", '<h2 id="pt-title-text"></h2><small id="pt-title-note"></small>');
  title.id = "pt-title";
  const live = studioNode("section", "pt-page pt-live", '<div class="tiles" id="pt-live-tiles"></div><div class="pt-rooms" id="pt-live-rooms"></div><p class="endline" id="pt-live-end"></p>');
  live.id = "pt-live";
  live.setAttribute("aria-label", "Live");
  const you = studioNode("section", "pt-page pt-you", youMarkup());
  you.id = "pt-you";
  you.setAttribute("aria-label", "You");
  const profile = studioNode("section", "pt-profile", profileMarkup());
  profile.id = "pt-profile";
  profile.setAttribute("aria-label", "Edit profile");
  const scrim = el("div", "pt-scrim");
  scrim.id = "pt-scrim";
  const hold = studioNode("section", "pt-sheet pt-hold", '<div class="rx" id="pt-hold-rx"></div><div class="pt-sheet-rows" id="pt-hold-rows"></div>');
  hold.id = "pt-hold";
  hold.setAttribute("aria-label", "Message actions");
  const pick = studioNode("section", "pt-sheet pt-pick", '<div class="pt-pick-step" id="pt-pick-step"></div><input type="datetime-local" class="pt-pick-time" id="pt-pick-time" tabindex="-1" aria-hidden="true">');
  pick.id = "pt-pick";
  pick.setAttribute("role", "dialog");
  const cue = studioNode("span", "pt-reply-cue", '<span class="rk"><i data-i="reply"></i></span>');
  cue.id = "pt-reply-cue";
  const bar = studioNode("nav", "hzn", horizonMarkup());
  bar.id = "horizon";
  bar.setAttribute("aria-label", "Places");
  const room = studioNode("section", "pt-room", roomMarkup());
  room.id = "pt-room";
  room.setAttribute("aria-label", "Voice room");
  const search = studioNode("section", "pt-sheet pt-search", '<div class="pt-search-top" id="pt-search-top"></div><div class="pt-search-body" id="pt-search-body"></div>');
  search.id = "pt-search";
  search.setAttribute("aria-label", "Search");
  const voiceScrim = el("div", "pt-voice-scrim");
  voiceScrim.id = "pt-voice-scrim";
  const voiceSheet = studioNode("section", "pt-voice-sheet", voiceSheetMarkup());
  voiceSheet.id = "pt-voice-sheet";
  voiceSheet.setAttribute("role", "dialog");
  voiceSheet.setAttribute("aria-label", "Voice");
  app.append(title, live, you, profile, scrim, hold, pick, search, cue, room, voiceScrim, voiceSheet, bar);
  const watch = studioNode("div", "pt-watch", watchMarkup());
  watch.id = "pt-watch";
  $("stage-view").appendChild(watch);
  const invite = studioNode("div", "pt-invite", inviteMarkup());
  invite.id = "pt-invite";
  document.body.appendChild(invite);
  buildChatHeader();
  buildChatBar();
  buildHomeHead();
  buildMessagesHead();
  $("add-server-btn").dataset.ptLabel = "Add";
  $("discover-btn").dataset.ptLabel = "Discover";
}

function buildHomeHead() {
  const find = studioNode("button", "key sq cream pt-find", '<i data-i="search"></i>');
  find.type = "button";
  find.id = "pt-find";
  find.setAttribute("aria-label", "Search");
  document.querySelector(".sidebar").appendChild(find);
  $("server-name").after(studioNode("span", "pt-home-chev", '<i data-i="chevron-down"></i>'));
  const add = el("button", "pt-sm-add", "Create Channel");
  add.type = "button";
  add.id = "pt-sm-add";
  $("sm-settings").before(add);
}

function buildMessagesHead() {
  const compose = studioNode("button", "key sq cream pt-compose", '<i data-i="square-pen"></i>');
  compose.type = "button";
  compose.id = "pt-compose";
  compose.setAttribute("aria-label", "New message");
  compose.onclick = () => openNewDm();
  document.querySelector(".sidebar").appendChild(compose);
  $("dm-search").after(studioNode("span", "pt-dm-find", '<i data-i="search"></i>'));
  $("dm-search").dataset.ptTyped = $("dm-search").placeholder;
}

function paintMessagesWords() {
  const field = $("dm-search");
  field.placeholder = studioSlim() ? "Find a person or a chat" : field.dataset.ptTyped;
}

function waitCard(row) {
  row.classList.add("pt-wait");
  const note = row.querySelector(".friend-note");
  note.before(el("div", "pt-wait-line", "wants to be friends"));
  note.classList.toggle("hidden", !note.title);
  row.querySelector(".friend-yes").appendChild(el("span", "pt-yes-word", "Accept"));
  row.setAttribute("role", "button");
  row.onclick = e => {
    if (e.target.closest("button")) return;
    openMemberCard({ id: row.dataset.person, name: row.querySelector(".dm-name").textContent }, row);
  };
  return row;
}

function decorateWaits(list) {
  const cards = [...list.querySelectorAll(".friend-request")].map(waitCard);
  if (cards.length <= WAIT_CARDS) studioPhone.waitsOpen = false;
  const { shown, more } = waitCards(cards.length, studioPhone.waitsOpen);
  cards.forEach((card, at) => card.classList.toggle("pt-wait-held", at >= shown));
  list.prepend(...cards);
  if (!more) return;
  const rest = el("button", "pt-wait-rest", waitMoreWords(more));
  rest.type = "button";
  rest.onclick = () => {
    studioPhone.waitsOpen = true;
    for (const card of cards) card.classList.remove("pt-wait-held");
    rest.remove();
  };
  cards[cards.length - 1].after(rest);
}

function liveFriend(id) {
  for (const room of studioPhone.rooms || []) {
    if (room.streams.some(stream => stream.userId === id)) return room;
  }
  return null;
}

function decorateFriendRow(row) {
  const room = liveFriend(row.dataset.person);
  if (room) {
    row.querySelector(".dm-name").appendChild(el("span", "live", "Live"));
    row.querySelector(".dm-preview").textContent = `In ${room.channelName}, sharing a screen`;
  }
  row.classList.toggle("pt-off", Boolean(row.querySelector(".presence-dot.offline")));
  row.appendChild(studioNode("span", "pt-dm-side", '<i data-i="chevron-right"></i>'));
}

async function paintGroupFaces(row) {
  const summary = state.servers.find(server => server.id === row.dataset.dm);
  const key = `${row.dataset.dm}:${summary ? summary.member_count : 0}`;
  if (!studioPhone.groupFaces.has(key)) {
    const group = await invoke("get_server", { serverId: row.dataset.dm }).catch(() => null);
    const others = group ? group.members.filter(member => !state.user || member.id !== state.user.id) : [];
    studioPhone.groupFaces.set(key, others.slice(0, 2));
  }
  const people = studioPhone.groupFaces.get(key);
  const face = row.querySelector(".group-avatar");
  if (!row.isConnected || !face || people.length < 2) return;
  face.classList.add("pt-duo");
  face.replaceChildren(...people.map(person => makeAvatar(person.id, person.name)));
}

function decorateThreadRow(row, now) {
  const thread = (state.dmThreads || []).find(item => item.user === row.dataset.dm);
  const side = el("span", "pt-dm-side");
  const time = row.querySelector(".dm-time");
  if (time) {
    if (thread) time.textContent = listAge(thread.last_at, now);
    side.appendChild(time);
  }
  const count = unread.dms.get(row.dataset.dm) || (thread && thread.kind === "group" ? serverUnreadCount(row.dataset.dm) : 0);
  if (count) side.appendChild(el("span", "pt-count", badgeText(count)));
  row.appendChild(side);
  if (thread && thread.kind === "group") paintGroupFaces(row);
}

function decorateMessages() {
  const list = $("channel-list");
  const saved = list.querySelector(":scope > .saved-entry");
  if (!saved || saved.dataset.ptDone || !studioSlim() || !$("app").classList.contains("dm-mode")) return;
  saved.dataset.ptDone = "1";
  const now = Date.now();
  for (const label of list.querySelectorAll(":scope > .channel-group")) {
    const words = label.querySelector("span") || label;
    if (words.childElementCount === 0) words.textContent = listLabel(words.textContent);
  }
  decorateWaits(list);
  const first = list.querySelector(":scope > .channel-group");
  first.after(saved);
  const friendRows = [...list.querySelectorAll(":scope > .friend-row")];
  if (friendRows.length) list.append(friendRows[0].previousElementSibling, ...friendRows);
  friendRows.forEach(decorateFriendRow);
  for (const row of list.querySelectorAll(":scope > .dm-thread[data-dm]")) decorateThreadRow(row, now);
  for (const label of list.querySelectorAll(":scope > .channel-group")) {
    const next = label.nextElementSibling;
    const twice = label.previousElementSibling === saved && label.textContent === first.textContent;
    label.classList.toggle("pt-bare", twice || !next || next.classList.contains("channel-group"));
  }
}

function buildChatBar() {
  const menu = $("composer-menu");
  for (const extra of BAR_EXTRAS) {
    const row = studioNode("button", "pt-cm", `<i data-i="${extra.icon}"></i>${extra.label}`);
    row.type = "button";
    row.id = `pt-cm-${extra.id}`;
    row.setAttribute("role", "menuitem");
    row.onclick = () => {
      menu.classList.add("hidden");
      setTimeout(() => $(extra.press).click(), 0);
    };
    menu.appendChild(row);
  }
  $("reply-bar").prepend(studioNode("span", "pt-chip-i", '<i data-i="reply"></i>'));
  document.addEventListener("click", e => {
    if (!studioSlim() || !e.target.closest || !e.target.closest("#attach-btn")) return;
    if (!menu.classList.contains("hidden") || state.view !== "server" || canAttachHere() || canSendVoiceHere()) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    closeFloating("composer-menu");
    for (const item of menu.querySelectorAll('[id^="cm-"]')) item.classList.add("hidden");
    menu.classList.remove("hidden");
  }, true);
}

function buildChatHeader() {
  const header = document.querySelector(".chat-header");
  const sub = el("span", "pt-chat-sub");
  sub.id = "pt-chat-sub";
  const title = el("button", "pt-chat-title");
  title.id = "pt-chat-title";
  title.type = "button";
  title.setAttribute("aria-label", "Channel options");
  title.setAttribute("aria-haspopup", "dialog");
  const chip = studioNode("button", "key tap pt-room-chip", '<i data-i="bars"></i><span id="pt-room-chip-name"></span>');
  chip.id = "pt-room-chip";
  chip.type = "button";
  $("channel-name").after(studioNode("span", "pt-chev", '<i data-i="chevron-down"></i>'));
  $("phone-back").appendChild(studioNode("span", "pt-back-i", '<i data-i="chevron-left"></i>'));
  header.append(sub, title, chip);
  const banner = el("div", "pt-chat-ban");
  banner.id = "pt-chat-ban";
  document.querySelector(".main").appendChild(banner);
}

function paintChatBanner() {
  const node = $("pt-chat-ban");
  const ref = chatBanner({ view: state.view, channel: state.currentChannel });
  if (node.dataset.ref === ref) return;
  node.dataset.ref = ref;
  node.style.backgroundImage = "";
  document.querySelector(".main").classList.toggle("pt-has-ban", Boolean(ref));
  if (!ref) return;
  loadServerImage(ref).then(url => {
    if (url && node.dataset.ref === ref) node.style.backgroundImage = `url("${url}")`;
  });
}

const STUDIO_TYPER_FACES = 3;

function layChatPane() {
  const main = document.querySelector(".main");
  const box = $("messages");
  const shown = box.offsetHeight > 0;
  main.classList.toggle("pt-pane", shown);
  if (!shown) return;
  main.style.setProperty("--pt-pane-y", `${box.offsetTop}px`);
  main.style.setProperty("--pt-pane-h", `${box.offsetHeight}px`);
}

function paintReplyNames() {
  if (!studioSlim() || !state.currentServer) return;
  for (const name of $("messages").querySelectorAll(".reply-ref-name:not([data-pt-ink])")) {
    name.dataset.ptInk = "on";
    const member = state.currentServer.members.find(found => found.name === name.textContent);
    if (member && member.color) name.style.setProperty("--pt-ink", `#${member.color}`);
  }
}

function paintTyper() {
  const strip = $("typing-strip");
  const channel = state.view === "server" && state.currentChannel ? state.currentChannel.id : null;
  const typers = channel ? typingTracker.typers(channel) : [];
  const faces = typers.map(id => ({ id, member: currentServerMember(id) })).filter(typer => typer.member).slice(0, STUDIO_TYPER_FACES);
  const users = faces.map(typer => typer.id).join(" ");
  const old = strip.querySelector(".pt-typer");
  if (old && old.dataset.users === users) return;
  if (old) old.remove();
  if (!faces.length) return;
  const stack = el("span", "pt-typer");
  stack.dataset.users = users;
  for (const typer of faces) stack.appendChild(makeAvatar(typer.id, typer.member.name, "avatar"));
  strip.prepend(stack);
}

function horizonParts() {
  const bar = $("horizon");
  return {
    bar,
    wrap: bar.querySelector(".hzn-tabs"),
    tabs: [...bar.querySelectorAll(".tab")],
    paths: [...bar.querySelectorAll(".hzn-line path")],
    grad: bar.querySelector("#hzn-g"),
    hot: bar.querySelector("#hzn-hot"),
    sun: bar.querySelector(".hzn-sun:not(.warm)"),
    warm: bar.querySelector(".hzn-sun.warm"),
    mic: bar.querySelector(".hzn-voice .mic"),
    hang: bar.querySelector(".hzn-voice .hang"),
    hangs: [...bar.querySelectorAll(".hang"), ...document.querySelectorAll("#pt-watch .hang")],
    room: bar.querySelector(".hzn-room"),
    open: bar.querySelector(".open"),
    badge: bar.querySelector(".tab .badge"),
    dot: bar.querySelector(".tab .ld"),
  };
}

function drawHorizon() {
  const h = horizonParts();
  const width = h.bar.clientWidth;
  if (!width) return;
  const pingAt = studioPhone.centres[1] ?? null;
  const room = h.bar.classList.contains("rm");
  const hot = room ? HORIZON.roomHot : HORIZON.hot;
  const d = room
    ? horizonPath(width, studioPhone.centre, studioPhone.swell, null, 0, HORIZON.roomSpread)
    : horizonPath(width, studioPhone.centre, studioPhone.swell, pingAt, studioPhone.ping);
  for (const path of h.paths) path.setAttribute("d", d);
  h.grad.setAttribute("x2", width);
  h.hot.setAttribute("x1", (studioPhone.centre - hot).toFixed(1));
  h.hot.setAttribute("x2", (studioPhone.centre + hot).toFixed(1));
  h.sun.style.setProperty("--x", `${studioPhone.centre.toFixed(1)}px`);
  h.warm.style.setProperty("--x", `${(pingAt ?? 0).toFixed(1)}px`);
  h.warm.style.opacity = Math.max(0, Math.min(1, studioPhone.ping / 5)).toFixed(3);
  h.tabs.forEach((tab, i) => {
    const lift = horizonLift(studioPhone.centres[i], studioPhone.centre, studioPhone.swell) - (i === 1 ? studioPhone.ping : 0);
    tab.style.setProperty("--lift", `${lift.toFixed(2)}px`);
  });
}

function measureHorizon() {
  const h = horizonParts();
  if (h.bar.classList.contains("rm")) return;
  const offset = h.wrap.offsetLeft;
  studioPhone.centres = h.tabs.map(tab => offset + tab.offsetLeft + tab.offsetWidth / 2);
  h.bar.classList.toggle("tight", h.tabs[0].offsetWidth < 56);
}

function horizonTabIndex() {
  return Math.max(0, STUDIO_TABS.findIndex(tab => tab.id === studioPhone.tab));
}

function horizonTarget() {
  const h = horizonParts();
  if (!h.bar.classList.contains("rm")) return studioPhone.centres[horizonTabIndex()] ?? 0;
  const slot = h.room.querySelector(".k.big");
  return h.room.offsetLeft + slot.offsetLeft + slot.offsetWidth / 2;
}

function layHorizon() {
  if (!studioSlim() || !$("horizon").clientWidth) return;
  measureHorizon();
  studioPhone.centre = horizonTarget();
  if (studioPhone.springs.slide) studioPhone.springs.slide = { ...studioPhone.springs.slide, x: studioPhone.centre, v: 0, target: studioPhone.centre, settled: true };
  drawHorizon();
}

function horizonSpring(name, change) {
  const base = studioPhone.springs[name] || { ...HORIZON_SPRINGS[name], x: 0, v: 0, target: 0, settled: true };
  studioPhone.springs[name] = { ...base, ...change, settled: false };
  if (studioPhone.calm) {
    studioPhone.springs[name] = { ...studioPhone.springs[name], x: studioPhone.springs[name].target, v: 0, settled: true };
    applyHorizonSprings();
    return;
  }
  if (studioPhone.frame) return;
  studioPhone.frameAt = performance.now();
  studioPhone.frame = requestAnimationFrame(stepHorizon);
}

function applyHorizonSprings() {
  const { slide, hop, knock } = studioPhone.springs;
  if (slide) studioPhone.centre = slide.x;
  if (hop) studioPhone.swell = hop.x;
  if (knock) studioPhone.ping = knock.x;
  drawHorizon();
}

function stepHorizon(now) {
  const seconds = (now - studioPhone.frameAt) / 1000;
  studioPhone.frameAt = now;
  let moving = false;
  for (const name of Object.keys(studioPhone.springs)) {
    const spring = studioPhone.springs[name];
    if (spring.settled) continue;
    studioPhone.springs[name] = springStep(spring, seconds);
    if (!studioPhone.springs[name].settled) moving = true;
  }
  if (studioPhone.followUntil > now) {
    measureHorizon();
    const target = horizonTarget();
    const slide = studioPhone.springs.slide || { ...HORIZON_SPRINGS.slide, x: target, v: 0 };
    studioPhone.springs.slide = { ...slide, target, settled: false };
    moving = true;
  }
  applyHorizonSprings();
  studioPhone.frame = moving ? requestAnimationFrame(stepHorizon) : 0;
}

function paintHorizonTab(again) {
  const h = horizonParts();
  h.tabs.forEach(tab => {
    const on = tab.dataset.tab === studioPhone.tab;
    tab.classList.toggle("on", on);
    tab.setAttribute("aria-selected", String(on));
    tab.querySelector(".i").classList.toggle("on", on);
  });
  if (!h.bar.clientWidth) return;
  measureHorizon();
  if (again) horizonSpring("hop", { x: studioPhone.swell, v: 190, target: HORIZON.swell });
  else horizonSpring("slide", { x: studioPhone.centre, target: horizonTarget() });
}

function studioRailServer() {
  const servers = railServers();
  return servers.find(server => server.id === studioPhone.lastServer) || servers[0] || null;
}

async function setPhoneTab(tab) {
  const again = tab === studioPhone.tab;
  disarmHangUp();
  studioPhone.tab = tab;
  $("app").dataset.tab = tab;
  paintHorizonTab(again);
  paintStudioTitle();
  if (tab === "live") refreshLiveRooms(true);
  if (tab === "you") paintYouPage();
  if (again) {
    const scroller = tab === "live" ? $("pt-live") : tab === "you" ? $("pt-you") : $("channel-list");
    scroller.scrollTo({ top: 0, behavior: studioPhone.calm ? "auto" : "smooth" });
    return;
  }
  if (tab === "messages" && state.view !== "dm") await enterDmMode(false);
  if (tab === "home" && (state.view !== "server" || (state.currentServer && state.currentServer.group))) {
    const server = studioRailServer();
    if (server) await selectServer(server.id);
  }
}

function paintStudioTitle() {
  const tab = studioPhone.tab;
  $("pt-title-text").textContent = STUDIO_TITLES[tab] || "";
  const streams = liveStreams(studioPhone.rooms).length;
  $("pt-title-note").textContent = tab === "live" ? liveSummary(streams, studioPhone.rooms.length) : "";
}

function syncPhoneTab() {
  const app = $("app");
  const view = app.classList.contains("dm-mode") ? "dm" : "server";
  if (view === "server" && state.currentServer && !state.currentServer.group) studioPhone.lastServer = state.currentServer.id;
  const next = phoneTabAfter(studioPhone.tab, { pane: phonePane(), view });
  if (next !== studioPhone.tab || app.dataset.tab !== next) {
    studioPhone.tab = next;
    app.dataset.tab = next;
    paintHorizonTab(false);
    paintStudioTitle();
  }
  if (phonePane() !== "list" && app.classList.contains("pt-voice-open")) app.classList.remove("pt-voice-open");
  if (studioPhone.searchHome && !app.classList.contains("pt-search-open")) stowSearchSheet();
  paintHomeHead();
  paintMessagesWords();
  if (phonePane() !== "chat" && app.classList.contains("pt-stream-full")) app.classList.remove("pt-stream-full");
  paintChatHeader();
  syncRoomMode();
  syncStreamFull();
}

function armHangUp() {
  const h = horizonParts();
  studioPhone.armed = true;
  h.bar.classList.add("armed");
  $("pt-watch").classList.add("armed");
  paintWatchField();
  for (const hang of h.hangs) {
    hang.classList.add("is-down");
    hang.setAttribute("aria-label", "Tap again to leave");
  }
  clearTimeout(studioPhone.fuse);
  studioPhone.fuse = setTimeout(disarmHangUp, HANG_UP_FUSE_MS);
}

function disarmHangUp() {
  if (!studioPhone.armed) return;
  const h = horizonParts();
  studioPhone.armed = false;
  clearTimeout(studioPhone.fuse);
  h.bar.classList.remove("armed");
  $("pt-watch").classList.remove("armed");
  paintWatchField();
  for (const hang of h.hangs) {
    hang.classList.remove("is-down");
    hang.setAttribute("aria-label", "Hang up");
  }
}

const STUDIO_SHEETS = ["pt-voice-open", "pt-hold-open", "pt-pick-open", "pt-chat-menu", "pt-search-open"];

function openSheet(name) {
  const app = $("app");
  for (const other of STUDIO_SHEETS) if (other !== name && app.classList.contains(other)) app.classList.remove(other);
  if (app.classList.contains(name)) return;
  app.classList.add(name);
  if (!(history.state && history.state.ptSheet)) history.pushState({ pane: phonePane(), ptSheet: name }, "");
}

function sheetOpen() {
  const app = $("app");
  return STUDIO_SHEETS.some(name => app.classList.contains(name));
}

function dropSheets() {
  const app = $("app");
  for (const name of STUDIO_SHEETS) if (app.classList.contains(name)) app.classList.remove(name);
  for (const held of document.querySelectorAll(".msg.pt-held")) held.classList.remove("pt-held");
}

function closeSheets(then) {
  const wasOpen = sheetOpen();
  dropSheets();
  if (wasOpen && history.state && history.state.ptSheet) {
    if (then) window.addEventListener("popstate", () => then(), { once: true });
    if (history.state.ptTray) {
      studioPhone.trayStep = false;
      history.go(-2);
    } else history.back();
    return;
  }
  if (then) then();
}

function openSearchSheet() {
  const box = $("search-box");
  const pop = $("search-pop");
  if (studioPhone.searchHome || box.classList.contains("hidden")) return;
  studioPhone.searchHome = { boxParent: box.parentNode, boxNext: box.nextSibling, popParent: pop.parentNode, popNext: pop.nextSibling };
  $("pt-search-top").appendChild(box);
  $("pt-search-body").appendChild(pop);
  openSheet("pt-search-open");
  if (searchScopes(searchScopeContext()).some(scope => scope.key === "server" && scope.enabled)) setSearchScope("server");
  showChosenScope();
  $("search-input").focus();
}

function showChosenScope() {
  const chosen = document.querySelector("#pt-search-body .search-scope-btn.active");
  if (!chosen) return;
  const bar = chosen.parentNode;
  bar.scrollLeft = Math.max(0, chosen.offsetLeft + chosen.offsetWidth - bar.clientWidth + 8);
}

function stowSearchSheet() {
  const home = studioPhone.searchHome;
  if (!home) return;
  studioPhone.searchHome = null;
  resetSearch();
  $("search-input").blur();
  home.boxParent.insertBefore($("search-box"), home.boxNext);
  home.popParent.insertBefore($("search-pop"), home.popNext);
}

function wireHomeHead() {
  $("pt-find").onclick = openSearchSheet;
  const openMenu = () => openServerMenu(state.currentServer, 0, 0);
  $("server-name-header").addEventListener("click", e => {
    if (e.target.closest("button") || !paintHomeHead()) return;
    openMenu();
  });
  $("server-name").addEventListener("keydown", e => {
    if ((e.key !== "Enter" && e.key !== " ") || !paintHomeHead()) return;
    e.preventDefault();
    openMenu();
  });
  $("pt-sm-add").onclick = () => {
    closeFloating();
    openCreateChannel("text", null);
  };
  new MutationObserver(() => {
    const server = state.currentServer;
    const mine = Boolean(server && menuServer && menuServer.id === server.id && server.can_manage_channels);
    $("pt-sm-add").classList.toggle("hidden", !mine);
  }).observe($("server-menu"), { attributes: true, attributeFilter: ["class"] });
  $("pt-search").addEventListener("click", e => {
    const row = e.target.closest(".search-row");
    if (!row || studioPhone.searchPick) return;
    e.preventDefault();
    e.stopPropagation();
    closeSheets(() => {
      studioPhone.searchPick = true;
      row.click();
      studioPhone.searchPick = false;
      if (phonePane() === "list") phoneStep("open");
    });
  }, true);
  new MutationObserver(() => {
    if (studioPhone.searchHome && $("search-pop").classList.contains("hidden") && $("app").classList.contains("pt-search-open")) closeSheets();
    else if (studioPhone.searchHome) showChosenScope();
  }).observe($("search-pop"), { attributes: true, attributeFilter: ["class"], childList: true, subtree: true });
}

function closeVoiceSheet() {
  if ($("app").classList.contains("pt-voice-open")) closeSheets();
}

function syncHorizonVoice() {
  const h = horizonParts();
  const inVoice = !$("phone-voice").classList.contains("hidden");
  const muted = $("phone-voice-mute").classList.contains("danger");
  const name = $("phone-voice-name").textContent;
  if (h.bar.classList.contains("in-voice") !== inVoice) {
    h.bar.classList.toggle("in-voice", inVoice);
    $("app").classList.toggle("pt-in-voice", inVoice);
    disarmHangUp();
    if (!inVoice) closeVoiceSheet();
    paintChatHeader();
    if (studioPhone.calm) layHorizon();
    else {
      studioPhone.followUntil = performance.now() + 900;
      horizonSpring("slide", { x: studioPhone.centre, target: studioPhone.centre });
    }
    decorateRooms();
  }
  h.mic.classList.toggle("muted", muted);
  h.mic.setAttribute("aria-label", muted ? "Unmute" : "Mute");
  if (h.mic.dataset.shown !== String(muted)) {
    h.mic.dataset.shown = String(muted);
    h.mic.querySelector(".i").outerHTML = StudioIcons.markup(muted ? "mic-off" : "mic");
  }
  h.open.querySelector(".open-name").textContent = name;
  h.open.setAttribute("aria-label", name ? `Open ${name}` : "Open the voice room");
  syncRoomMode();
  queueRoomPaint();
}

function syncHorizonBadge() {
  const h = horizonParts();
  const count = Number(($("home-badge") && $("home-badge").textContent) || 0);
  const text = badgeText(unreadChats([...unread.dms.values(), ...state.servers.filter(server => server.group).map(server => serverUnreadCount(server.id))]));
  h.badge.textContent = text;
  h.badge.classList.toggle("hidden", !text);
  if (count > studioPhone.badge && studioSlim() && !studioPhone.calm) {
    h.badge.classList.remove("pop");
    void h.badge.offsetWidth;
    h.badge.classList.add("pop");
    horizonSpring("knock", { x: studioPhone.ping, v: 150, target: 0 });
  }
  studioPhone.badge = count;
}

function decorateRail() {
  const waiting = [...document.querySelectorAll("#server-list .server-icon.waiting")];
  for (const item of document.querySelectorAll("#server-list .rail-item")) {
    const button = item.querySelector(".server-icon");
    if (!button || item.querySelector(".pt-name")) continue;
    const wait = joinWaits[waiting.indexOf(button)];
    const name = el("span", "pt-name", wait ? wait.name : button.title);
    if (button.classList.contains("active")) name.classList.add("on");
    item.appendChild(name);
    button.classList.toggle("pt-ink", serverTintIsLight(button.style.getPropertyValue("--server-tint")));
  }
  for (const head of document.querySelectorAll("#server-list .folder-head")) {
    if (head.parentNode.querySelector(":scope > .pt-name")) continue;
    head.after(el("span", "pt-name", head.title));
  }
  placeTileName();
}

function tileLabelWidth(cell) {
  const name = cell.querySelector(":scope > .pt-name");
  if (name) return name.offsetWidth;
  const drawn = getComputedStyle(cell, "::after");
  const words = /^"(.*)"$/.exec(drawn.content);
  if (!words) return 0;
  const pen = document.createElement("canvas").getContext("2d");
  pen.font = drawn.font;
  return pen.measureText(words[1]).width;
}

function placeTileName() {
  const name = document.querySelector("#server-list .rail-item > .pt-name.on");
  if (!name) return;
  const item = name.parentNode;
  const cell = item.offsetWidth;
  const next = item.nextElementSibling || (item.parentNode.id === "server-list" ? $("add-server-btn") : null);
  const sides = [item.previousElementSibling, next].filter(Boolean).map(side => (cell - tileLabelWidth(side)) / 2);
  name.style.maxWidth = cell ? `${tileNameWidth(cell, item.offsetLeft - TILE_NAME.margin, sides)}px` : "";
}

function personStack(people, className = "stack", max = STACK_MAX) {
  const { shown, more } = stackPeople(people, max);
  const stack = el("span", className);
  for (const person of shown) stack.appendChild(makeAvatar(person.id || person.name, person.name, "avatar"));
  if (more) stack.appendChild(el("span", "stack-more", `+${more}`));
  return stack;
}

function paintHomeWords() {
  const slim = studioSlim();
  for (const node of $("channel-list").querySelectorAll(".sidebar-link .channel-label, .section-title")) {
    const typed = node.dataset.ptTyped || node.textContent;
    if (!Object.hasOwn(STUDIO_HOME_WORDS, typed)) continue;
    node.dataset.ptTyped = typed;
    const shown = slim ? STUDIO_HOME_WORDS[typed] : typed;
    if (node.textContent !== shown) node.textContent = shown;
  }
}

function paintHomeHead() {
  const name = $("server-name");
  const menu = studioSlim() && studioPhone.tab === "home" && state.view === "server" && Boolean(state.currentServer) && !state.currentServer.group;
  if (menu) {
    name.setAttribute("role", "button");
    name.setAttribute("aria-haspopup", "menu");
    name.tabIndex = 0;
  } else if (name.getAttribute("aria-haspopup")) {
    name.removeAttribute("role");
    name.removeAttribute("aria-haspopup");
    name.removeAttribute("tabindex");
  }
  return menu;
}

function decorateTextRows(list, server) {
  for (const row of list.querySelectorAll(".channel[data-channel]")) {
    const hash = row.querySelector(":scope > .hash");
    if (hash && !hash.querySelector(".pt-hash")) hash.appendChild(studioNode("span", "pt-hash", '<i data-i="hash"></i>'));
    row.querySelector(":scope > .pt-count")?.remove();
    const count = server ? shownUnread(server.id, row.dataset.channel) : 0;
    if (count) row.appendChild(el("span", "pt-count", badgeText(count)));
  }
}

function decorateRooms() {
  if (!studioLook()) return;
  const list = $("channel-list");
  for (const head of list.querySelectorAll(".section-head")) {
    head.classList.toggle("pt-community", Boolean(head.nextElementSibling && head.nextElementSibling.classList.contains("sidebar-link")));
  }
  const server = state.view === "server" ? state.currentServer : null;
  paintHomeWords();
  paintHomeHead();
  decorateTextRows(list, server);
  for (const row of list.querySelectorAll(".voice-channel[data-room]")) {
    row.querySelector(".pt-room-note")?.remove();
    row.querySelector(".stack")?.remove();
    const channel = server && server.channels.find(c => c.id === row.dataset.room);
    if (!channel) continue;
    const here = state.voiceChannelId === channel.id;
    const people = roomFaces(roomPeople(channel), here && state.user ? state.user.id : null);
    const line = roomLine(people.length, here);
    const sharing = liveRooms([server]).some(room => room.channelId === channel.id && room.streams.length > 0);
    row.classList.toggle("pt-busy", Boolean(line));
    const label = row.querySelector(".channel-label");
    label.querySelector(".live")?.remove();
    if (sharing) label.appendChild(el("span", "live", "Live"));
    if (line) {
      const note = el("span", "pt-room-note" + (here ? " yes" : ""), line);
      label.after(note);
    }
    if (people.length) row.appendChild(personStack(people));
  }
}

async function refreshLiveRooms(force) {
  if (studioPhone.roomsBusy || !state.user) return;
  if (!force && Date.now() - studioPhone.roomsAt < STUDIO_LIVE_REFRESH_MS) return;
  studioPhone.roomsBusy = true;
  try {
    const servers = [];
    for (const summary of railServers()) {
      const current = state.currentServer && state.currentServer.id === summary.id ? state.currentServer : null;
      servers.push(current || await invoke("get_server", { serverId: summary.id }).catch(() => null));
    }
    studioPhone.servers = servers.filter(Boolean);
    studioPhone.rooms = liveRooms(studioPhone.servers, { userId: state.user.id, channelId: state.voiceChannelId });
    studioPhone.roomsAt = Date.now();
  } finally {
    studioPhone.roomsBusy = false;
  }
  paintLivePage();
  queueRoomPaint();
}

function liveTile(stream, wide) {
  const room = stream.room;
  const tile = el("button", "tile" + (wide ? " wide" : ""));
  tile.type = "button";
  const thumb = el("span", "th theme-dark");
  thumb.style.setProperty("--tint", personTint(stream.userId));
  thumb.appendChild(makeAvatar(stream.userId, stream.name, "avatar th-avatar"));
  thumb.appendChild(el("span", "live", "Live"));
  if (room.here) thumb.appendChild(studioNode("span", "here", '<i data-i="volume-2"></i>You\'re in this room'));
  const foot = el("span", "ft");
  foot.appendChild(makeAvatar(stream.userId, stream.name, "avatar"));
  const text = el("span", "t");
  text.appendChild(el("b", "", stream.mine ? "Your screen" : `${stream.name}'s screen`));
  text.appendChild(el("span", "", `${room.channelName} · ${room.serverName}`));
  foot.appendChild(text);
  if (wide) foot.appendChild(personStack(room.people.filter(person => person.id !== stream.userId)));
  tile.append(thumb, foot);
  tile.onclick = () => openLiveStream(stream);
  return tile;
}

function liveRoomRow(room) {
  const row = el("button", "rw");
  row.type = "button";
  row.appendChild(studioNode("span", "rw-icon", '<i data-i="volume-2"></i>'));
  const text = el("span", "t");
  text.appendChild(el("b", "", room.channelName));
  text.appendChild(el("span", "", `${room.serverName} · ${roomLine(room.people.length, room.here)}`));
  row.append(text, personStack(room.people));
  row.onclick = () => openLiveRoom(room);
  return row;
}

function paintLivePage() {
  const streams = liveStreams(studioPhone.rooms);
  const quiet = studioPhone.rooms.filter(room => !room.streams.length);
  $("pt-live-tiles").replaceChildren(...streams.map((stream, i) => liveTile(stream, i === 0)));
  const rooms = $("pt-live-rooms");
  rooms.replaceChildren();
  rooms.classList.toggle("slab", quiet.length > 0);
  if (quiet.length) {
    rooms.appendChild(el("div", "sec lbl", "Rooms with people in them"));
    for (const room of quiet) rooms.appendChild(liveRoomRow(room));
  }
  $("pt-live-end").textContent = streams.length || quiet.length ? "That is everyone. For now." : "No one is in a room right now.";
  const dot = horizonParts().dot;
  dot.classList.toggle("hidden", !streams.length);
  paintStudioTitle();
}

async function openLiveRoom(room) {
  await selectServer(room.serverId);
  await setPhoneTab("home");
  const row = document.querySelector(`#channel-list .voice-channel[data-room="${CSS.escape(room.channelId)}"]`);
  if (row) row.scrollIntoView({ block: "center" });
}

async function openLiveStream(stream) {
  if (!state.currentServer || state.currentServer.id !== stream.room.serverId) await selectServer(stream.room.serverId);
  await watchScreen(stream.userId, stream.name);
}

function paintYouPage() {
  if (!state.user) return;
  const name = state.user.display_name || state.user.username;
  const avatar = $("pt-me-avatar");
  avatar.replaceChildren();
  paintAvatar(avatar, state.user.id, name);
  avatar.appendChild(el("span", "presence-dot " + (state.user.presence || "online")));
  $("pt-me-name").textContent = name;
  paintPresenceLine();
  const theme = document.documentElement.dataset.theme === "light" ? "light" : "dark";
  for (const key of document.querySelectorAll("#pt-you [data-theme-choice]")) {
    const on = key.dataset.themeChoice === theme;
    key.classList.toggle("is-down", on);
    key.setAttribute("aria-pressed", String(on));
  }
  $("pt-glass").setAttribute("aria-checked", String(document.documentElement.dataset.fx !== "lite"));
  const level = notifyPrefs().level;
  for (const row of document.querySelectorAll("#pt-you [data-notify]")) row.setAttribute("aria-checked", String(row.dataset.notify === level));
  $("pt-about-words").classList.toggle("hidden", !earlyBuild(updateStatus && updateStatus.current));
  $("pt-switch-account").classList.toggle("hidden", !accountRows(account).length);
}

function ownPresence() {
  return (state.user && state.user.presence) || "online";
}

function ownPresenceUntil() {
  const until = Number((state.voice && state.voice.presence_until) || (state.user && state.user.presence_until));
  return ownPresence() !== "online" && until && until * 1000 > Date.now() ? until : 0;
}

function paintPresenceLine() {
  const presence = ownPresence();
  const words = $("me-status").textContent;
  const until = ownPresenceUntil();
  const key = $("pt-presence");
  key.dataset.presence = presence;
  $("pt-me-status").textContent = presence === "do_not_disturb" && until ? PRESENCE_LABELS.do_not_disturb : words;
  const end = $("pt-me-until");
  end.textContent = until ? `\u00a0· until ${formatUntil(until)}` : "";
  end.classList.remove("hidden");
  if (until && key.scrollWidth > key.parentNode.clientWidth) end.classList.add("hidden");
  key.setAttribute("aria-label", `Presence: ${$("pt-me-status").textContent}${end.classList.contains("hidden") ? "" : end.textContent}. Change`);
}

function presenceChoices() {
  return [...document.querySelectorAll("#pp-presence button[data-presence]")].map(button => {
    const small = button.querySelector("small");
    return {
      presence: button.dataset.presence,
      label: button.querySelector("span:not(.presence-dot)").firstChild.textContent.trim(),
      hint: small ? small.dataset.text || small.textContent : "",
    };
  });
}

function presencePip(presence) {
  return `<span class="pdot ${presence}" aria-hidden="true"></span>`;
}

function presenceRow(choice, current, until) {
  const row = studioNode("button", "srow pick", `${presencePip(choice.presence)}<span class="l"></span><i data-i="check" class="tick"></i>`);
  row.type = "button";
  row.setAttribute("role", "radio");
  row.setAttribute("aria-checked", String(choice.presence === current));
  const label = row.querySelector(".l");
  label.textContent = choice.label;
  const hint = choice.presence === current && until ? `Until ${formatUntil(until)}` : choice.hint;
  if (hint) label.appendChild(el("small", "", hint));
  row.onclick = () => choice.presence === "online" ? closeSheets(() => setOwnPresence("online", null)) : showDurationStep(choice.presence);
  return row;
}

function durationChoices(presence) {
  openDurations({ kind: "presence", presence }, $("pt-presence"));
  const choices = [...document.querySelectorAll("#pp-durations button[data-mins]")].map(button => ({ mins: button.dataset.mins, label: button.textContent }));
  const note = $("pp-durations-note").textContent;
  closeDurations();
  return { choices, note };
}

function durationRow(icon, label, run, trailing = "") {
  const row = studioNode("button", "srow", `${icon}<span class="l"></span>${trailing}`);
  row.type = "button";
  row.querySelector(".l").textContent = label;
  row.onclick = run;
  return row;
}

async function setOwnPresence(presence, until) {
  await saveOwnProfile({ presence });
  await setStatusExpiry({ presence: until });
  paintYouPage();
}

function pickStep(build, direction) {
  const sheet = $("pt-pick");
  const step = $("pt-pick-step");
  const from = sheet.offsetHeight;
  step.replaceChildren(...build());
  StudioIcons.paint(step);
  if (!direction || !from) return;
  step.classList.remove("pt-step-in", "pt-step-back");
  void step.offsetWidth;
  step.classList.add(direction === "back" ? "pt-step-back" : "pt-step-in");
  const to = sheet.offsetHeight;
  if (to === from) return;
  sheet.style.height = `${from}px`;
  void sheet.offsetHeight;
  sheet.classList.add("pt-sizing");
  sheet.style.height = `${to}px`;
  const done = () => {
    sheet.classList.remove("pt-sizing");
    sheet.style.height = "";
  };
  sheet.addEventListener("transitionend", done, { once: true });
  setTimeout(done, 400);
}

function presenceRows() {
  const current = ownPresence();
  const until = ownPresenceUntil();
  return presenceChoices().map(choice => presenceRow(choice, current, until));
}

function showPresenceRows(direction) {
  pickStep(presenceRows, direction);
}

function showDurationStep(presence) {
  pickStep(() => {
    const { choices, note } = durationChoices(presence);
    const head = studioNode("div", "phead", `<button type="button" class="key sq tap back" aria-label="Back"><i data-i="chevron-left"></i></button><span class="sec">For how long</span><span class="who">${presencePip(presence)}<span></span></span>`);
    head.querySelector(".who > span:last-child").textContent = (presenceChoices().find(choice => choice.presence === presence) || {}).label || "";
    head.querySelector(".back").onclick = () => showPresenceRows("back");
    const clock = '<i data-i="clock"></i>';
    const rows = [];
    for (const choice of choices) {
      if (choice.mins === "never") {
        rows.push(durationRow('<i data-i="calendar-days"></i>', "Pick a time", () => pickPresenceTime(presence), '<i data-i="chevron-right" class="chev"></i>'));
        rows.push(durationRow(presencePip(presence), choice.label, () => closeSheets(() => setOwnPresence(presence, null))));
        continue;
      }
      rows.push(durationRow(clock, choice.label, () => closeSheets(() => setOwnPresence(presence, untilFromChoice(choice.mins)))));
    }
    const out = [head, ...rows];
    if (note) out.push(el("p", "hint", note));
    return out;
  }, "in");
}

function pickPresenceTime(presence) {
  pickTime(until => closeSheets(() => setOwnPresence(presence, until)));
}

function pickTime(use) {
  const input = $("pt-pick-time");
  input.value = toLocalInput(new Date(Date.now() + 2 * 3600e3));
  input.onchange = () => {
    const when = new Date(input.value).getTime();
    if (!when || when <= Date.now()) {
      toast("Pick a time in the future");
      return;
    }
    use(Math.floor(when / 1000));
  };
  try {
    input.showPicker();
  } catch {
    input.focus();
    input.click();
  }
}

function openPickSheet(label, build) {
  $("pt-pick").setAttribute("aria-label", label);
  pickStep(build);
  openSheet("pt-pick-open");
}

function openPresenceSheet() {
  openPickSheet("Presence", presenceRows);
}

function profileOpen() {
  return $("pt-profile").classList.contains("on");
}

function savedStatus() {
  const user = state.user || {};
  return { name: user.display_name || "", status: decodeCustomEmojis(user.custom_status), emoji: user.status_emoji || "" };
}

function profileDirty() {
  if (!profileOpen() || !state.user) return false;
  const saved = savedStatus();
  return $("pt-prof-dn").value !== saved.name
    || $("pt-prof-status").value !== saved.status
    || studioPhone.profileEmoji !== saved.emoji
    || studioPhone.profileClear !== undefined;
}

function paintProfileSave() {
  $("pt-prof-save").disabled = !profileDirty();
  const hasStatus = Boolean($("pt-prof-status").value.trim() || studioPhone.profileEmoji);
  $("pt-prof-clear-fld").classList.toggle("hidden", !hasStatus);
  const until = studioPhone.profileClear !== undefined ? studioPhone.profileClear : (state.voice && state.voice.status_until) || null;
  $("pt-prof-clear-v").textContent = until ? `Clears at ${formatUntil(until)}` : "Never";
}

function paintProfileEmoji() {
  const slot = $("pt-prof-emoji");
  const value = studioPhone.profileEmoji;
  paintEmojiSlot(slot, value);
  if (!value) slot.replaceChildren(studioNode("span", "none", '<i data-i="smile"></i>'));
}

function profileName() {
  return $("pt-prof-dn").value || (state.user && state.user.username) || "";
}

function paintProfileTop() {
  const user = state.user;
  const ban = $("pt-prof-ban");
  ban.dataset.banner = "";
  ban.classList.toggle("wash", !user.banner);
  if (user.banner) paintProfileBanner(ban, user.banner, user.node, null, bannerColor(user.id));
  else ban.style.background = "";
  paintCardEffect(ban, user.card_effect, user.id);
  $("pt-prof-ban-rm").classList.toggle("hidden", !user.banner);
  $("pt-prof-banner-words").textContent = user.banner ? "Change banner" : "Add banner";
  const avatar = $("pt-prof-av");
  avatar.replaceChildren();
  const face = makeAvatar(user.id, profileName(), "avatar");
  setAvatarBorder(face, user.avatar_border);
  face.appendChild(el("span", "presence-dot " + ownPresence()));
  avatar.appendChild(face);
  $("pt-prof-name").textContent = profileName();
  const sub = $("pt-prof-sub");
  sub.replaceChildren(document.createTextNode(user.username || ""));
  const pronouns = $("pt-prof-pro").value.trim();
  if (pronouns) sub.append(el("i"), document.createTextNode(pronouns));
}

function lookTile(label, preview, on, pick) {
  const tile = el("button", "look" + (on ? " on" : ""));
  tile.type = "button";
  tile.setAttribute("role", "radio");
  tile.setAttribute("aria-checked", String(on));
  tile.append(preview, el("small", "", label));
  tile.onclick = async () => {
    await pick();
    paintProfileLooks();
    paintProfileTop();
  };
  return tile;
}

function paintProfileLooks() {
  const user = state.user;
  const border = id => {
    const box = el("span", "avw");
    const face = makeAvatar(user.id, profileName(), "avatar");
    setAvatarBorder(face, id);
    box.appendChild(face);
    return box;
  };
  const effect = id => {
    const mini = el("span", "mini");
    if (user.banner) paintProfileBanner(mini, user.banner, user.node, null, bannerColor(user.id));
    else mini.classList.add("wash");
    paintCardEffect(mini, id, user.id);
    return mini;
  };
  $("pt-prof-borders").replaceChildren(
    lookTile("None", border(null), !user.avatar_border, () => saveProfileLook(null, user.card_effect || null)),
    ...AVATAR_BORDERS.map(b => lookTile(b.label, border(b.id), user.avatar_border === b.id, () => saveProfileLook(b.id, user.card_effect || null))),
  );
  const effects = [lookTile("None", effect(null), !user.card_effect, () => saveProfileLook(user.avatar_border || null, null)),
    ...CARD_EFFECTS.map(e => lookTile(e.label, effect(e.id), user.card_effect === e.id, () => saveProfileLook(user.avatar_border || null, e.id)))];
  for (const tile of effects) tile.classList.add("fxl");
  $("pt-prof-effects").replaceChildren(...effects);
}

function wornTag() {
  const worn = wornServerId(state.user);
  const server = (state.servers || []).find(s => s.id === worn && s.tag);
  return server ? { server, tag: server.tag } : null;
}

function paintProfileTag() {
  const value = $("pt-prof-tag-v");
  const worn = wornTag();
  const button = $("pt-prof-tag");
  button.querySelector(":scope > .server-tag")?.remove();
  value.textContent = worn ? worn.server.name : "None";
  if (worn) value.before(serverTagChip(worn.tag));
  const options = wearableTags(state.servers);
  button.disabled = !options.length && !worn;
}

function fillProfileEditor() {
  const user = state.user;
  const saved = savedStatus();
  $("pt-prof-dn").value = saved.name;
  $("pt-prof-pro").value = (state.voice && state.voice.pronouns) || "";
  $("pt-prof-status").value = saved.status;
  $("pt-prof-about").value = decodeCustomEmojis(user.about);
  $("pt-prof-tag-hint").textContent = $("account-server-tag").parentNode.querySelector(".setting-sub").textContent;
  studioPhone.profileEmoji = saved.emoji;
  studioPhone.profileClear = undefined;
  paintProfileAbout();
  paintProfileEmoji();
  paintProfileTop();
  paintProfileTag();
  paintProfileLooks();
  paintProfileSave();
}

function paintProfileAbout() {
  const about = $("pt-prof-about");
  $("pt-prof-count").textContent = `${about.value.length} / ${about.maxLength}`;
}

function paintProfileFades() {
  const scroll = $("pt-prof-scroll");
  scroll.classList.toggle("down", scroll.scrollTop > 2);
  scroll.classList.toggle("more", scroll.scrollTop + scroll.clientHeight < scroll.scrollHeight - 2);
}

function syncProfileStep(open) {
  const step = layerStep("ptProfile", open, studioPhone.profileStep, history.state);
  if (step === "push") history.pushState({ ...(history.state || { pane: phonePane() }), ptProfile: true }, "");
  studioPhone.profileStep = open;
  if (step === "back") history.back();
}

function openProfileEditor() {
  if (!state.user) return;
  fillProfileEditor();
  const page = $("pt-profile");
  page.classList.add("on");
  $("pt-prof-scroll").scrollTop = 0;
  paintProfileFades();
  syncProfileStep(true);
}

function closeProfileEditor() {
  $("pt-profile").classList.remove("on");
  studioPhone.profileClear = undefined;
  syncProfileStep(false);
}

function leaveProfileEditor() {
  if (!profileDirty()) return closeProfileEditor();
  openPickSheet("Save your changes?", () => {
    const ask = studioNode("div", "pt-ask", '<b>Save your changes?</b><div class="ks"><button type="button" class="key lg primary">Save</button><button type="button" class="key lg">Discard</button></div>');
    const [save, discard] = ask.querySelectorAll(".key");
    save.onclick = () => closeSheets(async () => {
      if (await saveProfileEditor()) closeProfileEditor();
    });
    discard.onclick = () => closeSheets(() => closeProfileEditor());
    return [ask];
  });
}

async function saveProfileEditor() {
  if (!profileDirty()) return true;
  const save = $("pt-prof-save");
  save.disabled = true;
  try {
    state.user = await invoke("update_profile", {
      displayName: $("pt-prof-dn").value || null,
      customStatus: encodeCustomEmojis($("pt-prof-status").value) || null,
      presence: ownPresence(),
      statusEmoji: studioPhone.profileEmoji,
    });
    if (studioPhone.profileClear !== undefined) await setStatusExpiry({ status: studioPhone.profileClear });
    studioPhone.profileClear = undefined;
    renderUserPanel();
    if (state.currentServer) await refreshCurrentServer();
    toast("Profile updated");
    fillProfileEditor();
    paintYouPage();
    return true;
  } catch (e) {
    toast(String(e));
    paintProfileSave();
    return false;
  }
}

function clearAfterRows() {
  openDurations({ kind: "status" }, $("pt-prof-clear"));
  const choices = [...document.querySelectorAll("#pp-durations button[data-mins]")].map(button => ({ mins: button.dataset.mins, label: button.textContent }));
  closeDurations();
  const current = studioPhone.profileClear !== undefined ? studioPhone.profileClear : (state.voice && state.voice.status_until) || null;
  const set = until => closeSheets(() => {
    studioPhone.profileClear = until;
    paintProfileSave();
  });
  const head = studioNode("div", "phead solo", '<span class="sec">Clear after</span>');
  const rows = [head];
  for (const choice of choices) {
    if (choice.mins === "never") {
      rows.push(durationRow('<i data-i="calendar-days"></i>', "Pick a time", () => pickTime(set), '<i data-i="chevron-right" class="chev"></i>'));
      const slot = el("span", "eslot");
      paintEmojiSlot(slot, studioPhone.profileEmoji);
      if (!studioPhone.profileEmoji) slot.replaceChildren(studioNode("span", "none", '<i data-i="smile"></i>'));
      const never = durationRow("", choice.label, () => set(null), '<i data-i="check" class="tick"></i>');
      never.prepend(slot);
      never.classList.add("pick");
      never.setAttribute("aria-checked", String(!current));
      rows.push(never);
      continue;
    }
    rows.push(durationRow('<i data-i="clock"></i>', choice.label, () => set(untilFromChoice(choice.mins))));
  }
  return rows;
}

function serverTagRows() {
  const worn = wornServerId(state.user);
  const row = (value, chip, label) => {
    const pick = studioNode("button", "srow pick", '<span class="l"></span><i data-i="check" class="tick"></i>');
    pick.type = "button";
    pick.setAttribute("role", "radio");
    pick.setAttribute("aria-checked", String(value === worn));
    pick.querySelector(".l").textContent = label;
    if (chip) pick.prepend(chip);
    pick.onclick = () => closeSheets(() => wearTag(value));
    return pick;
  };
  const servers = (state.servers || []).filter(s => s && s.tag && !s.group).sort((a, b) => a.name.localeCompare(b.name));
  return [row(NO_SERVER_TAG, null, "None"), ...servers.map(s => row(s.id, serverTagChip(s.tag), s.name))];
}

async function wearTag(value) {
  try {
    state.user = await invoke("wear_server_tag", { serverId: value || null });
    toast(value ? "You are wearing that tag" : "Server tag hidden");
    if (state.currentServer) await refreshCurrentServer();
  } catch (err) {
    toast(String(err));
  }
  paintProfileTag();
}

async function chooseProfileBanner() {
  try {
    const src = await chooseImage("Choose a profile banner", $("server-image-file"));
    if (src) await saveAccountBanner(await cropImageFile(src, 600, 240));
  } catch (err) {
    toast(String(err));
  }
  paintProfileTop();
  paintProfileLooks();
}

function wireProfileEditor() {
  $("pt-prof-back").onclick = leaveProfileEditor;
  $("pt-prof-scroll").addEventListener("scroll", paintProfileFades, { passive: true });
  $("pt-prof-save").onclick = () => saveProfileEditor();
  $("pt-prof-dn").addEventListener("input", () => {
    $("pt-prof-name").textContent = profileName();
    paintProfileSave();
  });
  $("pt-prof-status").addEventListener("input", paintProfileSave);
  $("pt-prof-pro").addEventListener("change", async e => {
    try {
      state.voice = await invoke("set_pronouns", { pronouns: e.target.value });
      e.target.value = state.voice.pronouns || "";
      toast(state.voice.pronouns ? "Pronouns saved" : "Pronouns removed");
    } catch (err) {
      toast(String(err));
    }
    paintProfileTop();
  });
  $("pt-prof-about").addEventListener("input", paintProfileAbout);
  $("pt-prof-about").addEventListener("change", async e => {
    try {
      state.user = await invoke("set_about", { about: encodeCustomEmojis(e.target.value) });
      toast("About Me saved");
    } catch (err) {
      toast(String(err));
    }
  });
  $("pt-prof-about-emoji").onclick = e => openEmojiPop(e.currentTarget, picked => {
    insertEmojiInName($("pt-prof-about"), picked);
    $("pt-prof-about").dispatchEvent(new Event("change", { bubbles: true }));
  }, { custom: true });
  $("pt-prof-emoji").onclick = e => {
    const set = emoji => {
      studioPhone.profileEmoji = emoji;
      paintProfileEmoji();
      paintProfileSave();
    };
    openEmojiPop(e.currentTarget, set, { custom: true, onClear: studioPhone.profileEmoji ? () => set("") : null });
  };
  $("pt-prof-clear").onclick = () => openPickSheet("Clear after", clearAfterRows);
  $("pt-prof-tag").onclick = () => openPickSheet("Server tag", serverTagRows);
  $("pt-prof-avatar").onclick = e => {
    e.stopPropagation();
    openAvatarMenu(e.currentTarget);
  };
  $("pt-prof-banner").onclick = chooseProfileBanner;
  $("pt-prof-ban-rm").onclick = async () => {
    await saveAccountBanner(null);
    paintProfileTop();
    paintProfileLooks();
  };
  window.addEventListener("popstate", () => {
    if (!profileOpen() || (history.state && (history.state.ptProfile || history.state.ptSheet))) return;
    if (profileDirty()) {
      history.pushState({ ...(history.state || { pane: phonePane() }), ptProfile: true }, "");
      leaveProfileEditor();
      return;
    }
    studioPhone.profileStep = false;
    closeProfileEditor();
  });
}

function wireYouPage() {
  wireProfileEditor();
  $("pt-edit").onclick = e => {
    e.stopPropagation();
    openProfileEditor();
  };
  $("pt-settings").onclick = () => $("settings-btn").click();
  $("pt-presence").onclick = openPresenceSheet;
  $("pt-switch-account").onclick = () => openAccounts();
  $("pt-sign-out").onclick = () => signOutOfCurrent();
  if (typeof Star !== "undefined") Star.mount($("pt-about-star"));
  for (const row of document.querySelectorAll("#pt-you [data-notify]")) {
    row.onclick = () => {
      const select = $("notify-level");
      select.value = row.dataset.notify;
      select.dispatchEvent(new Event("change", { bubbles: true }));
      paintYouPage();
    };
  }
  $("pt-glass").onclick = () => {
    setLookFx(document.documentElement.dataset.fx === "lite" ? "glass" : "lite");
    paintYouPage();
  };
  for (const key of document.querySelectorAll("#pt-you [data-theme-choice]")) {
    key.onclick = () => {
      const select = $("theme-select");
      select.value = key.dataset.themeChoice;
      select.dispatchEvent(new Event("change", { bubbles: true }));
      paintYouPage();
    };
  }
}

function wireHorizon() {
  const h = horizonParts();
  h.bar.addEventListener("click", e => {
    const tab = e.target.closest(".tab");
    if (tab) return setPhoneTab(tab.dataset.tab);
    if (e.target.closest(".hang")) {
      if (hangUpTap(studioPhone.armed) === "leave") {
        disarmHangUp();
        disconnectVoice();
      } else armHangUp();
      return;
    }
    const roomKey = e.target.closest("[data-room-key]");
    if (roomKey) {
      disarmHangUp();
      pressRoomKey(roomKey.dataset.roomKey);
      return;
    }
    if (e.target.closest(".mic")) {
      disarmHangUp();
      $("mute-btn").click();
      return;
    }
    if (e.target.closest(".open")) {
      if ($("app").classList.contains("pt-voice-open")) closeSheets();
      else openSheet("pt-voice-open");
    }
  });
  document.addEventListener("pointerdown", e => {
    if (studioPhone.armed && !h.hangs.some(hang => hang.contains(e.target))) disarmHangUp();
    studioPhone.soundsWereOpen = !$("soundboard-pop").classList.contains("hidden");
    studioPhone.trayWasOpen = trayShown();
  });
  window.addEventListener("mousedown", e => {
    if (studioSlim() && trayShown() && trayTap(true, trayPlace(e.target)) === "stay") e.stopPropagation();
  }, true);
  window.addEventListener("click", e => {
    if (!e.isTrusted) return;
    const wasOpen = studioPhone.trayWasOpen;
    studioPhone.trayWasOpen = false;
    if (!studioSlim() || trayTap(wasOpen, trayPlace(e.target)) !== "close") return;
    e.preventDefault();
    e.stopPropagation();
    $("soundboard-pop").classList.toggle("hidden", true);
  }, true);
  document.addEventListener("keydown", e => {
    if (e.key === "Escape") {
      disarmHangUp();
      closeSheets();
    }
  });
  $("pt-scrim").onclick = () => closeSheets();
  window.addEventListener("popstate", () => {
    const back = history.state && history.state.ptSheet;
    if (!(history.state && history.state.ptTray)) {
      studioPhone.trayStep = false;
      if (trayShown()) $("soundboard-pop").classList.toggle("hidden", true);
    }
    dropSheets();
    if (back === "pt-voice-open" && phonePane() === "list" && !$("phone-voice").classList.contains("hidden")) $("app").classList.add(back);
  });
  $("channel-list").addEventListener("click", e => {
    if (!studioSlim() || e.target.closest("button")) return;
    if (!e.target.closest(".voice-channel.connected")) return;
    e.stopPropagation();
    e.preventDefault();
    openSheet("pt-voice-open");
  }, true);
}

function watchStudioPhone() {
  const watch = (node, options, fn) => new MutationObserver(fn).observe(node, options);
  watch($("app"), { attributes: true, attributeFilter: ["class", "data-pane"] }, syncPhoneTab);
  watch($("phone-voice"), { attributes: true, attributeFilter: ["class"], subtree: true, childList: true, characterData: true }, syncHorizonVoice);
  watch($("home-btn"), { childList: true, subtree: true, characterData: true }, syncHorizonBadge);
  watch($("server-list"), { childList: true, subtree: true }, () => {
    decorateRail();
    refreshLiveRooms(false);
  });
  watch($("channel-name"), { childList: true, characterData: true, subtree: true }, paintChatHeader);
  watch($("chat-presence"), { attributes: true, attributeFilter: ["class", "title"] }, paintChatHeader);
  watch($("channel-list"), { childList: true }, () => {
    if (studioPhone.rowOpen && !studioPhone.rowOpen.row.isConnected) studioPhone.rowOpen = null;
    decorateRooms();
    decorateMessages();
    refreshLiveRooms(false);
  });
  watch($("voice-panel"), { attributes: true, attributeFilter: ["class", "data-quality"], subtree: true, childList: true }, queueRoomPaint);
  watch($("soundboard-pop"), { attributes: true, attributeFilter: ["class"] }, () => {
    syncRoomKeys();
    decorateTray();
  });
  watch($("video-pane"), { attributes: true, attributeFilter: ["class"] }, syncStreamFull);
  watch($("video-label"), { childList: true, characterData: true, subtree: true }, paintWatch);
  watch($("messages"), { childList: true }, paintWatch);
  watch($("messages"), { childList: true }, paintReplyNames);
  watch($("typing-text"), { childList: true, characterData: true, subtree: true }, paintTyper);
  new ResizeObserver(layChatPane).observe($("messages"));
  window.addEventListener("resize", layChatPane);
  watch($("me-status"), { childList: true, characterData: true, subtree: true }, () => studioPhone.tab === "you" && paintYouPage());
  watch(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "data-fit", "data-fx"] }, () => {
    paintYouPage();
    paintHomeWords();
    paintHomeHead();
    placeTileName();
    decorateTray();
    paintMessagesWords();
    if ($("app").classList.contains("dm-mode")) renderDmThreads();
    layHorizon();
  });
  window.addEventListener("resize", layHorizon);
  document.addEventListener("visibilitychange", () => !document.hidden && refreshLiveRooms(false));
  studioPhone.liveTimer = setInterval(() => !document.hidden && studioSlim() && refreshLiveRooms(false), STUDIO_LIVE_REFRESH_MS);
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(layHorizon);
}

function paintChatHeader() {
  const sub = $("pt-chat-sub");
  if (!sub) return;
  const presence = $("chat-presence").classList.contains("hidden") ? "" : $("chat-presence").title;
  const text = chatSubtitle({
    view: state.view === "dm" ? "dm" : "server",
    serverName: state.currentServer ? state.currentServer.name : "",
    group: Boolean(state.view === "server" && state.currentServer && state.currentServer.group),
    presence,
  });
  if (sub.textContent !== text) sub.textContent = text;
  paintChatBanner();
  const name = $("phone-voice-name").textContent;
  const inVoice = !$("phone-voice").classList.contains("hidden");
  $("pt-room-chip").classList.toggle("hidden", !inVoice);
  if ($("pt-room-chip-name").textContent !== name) $("pt-room-chip-name").textContent = name;
}

function backToRoom() {
  if (phonePane() !== "list") phoneStep("back");
  setTimeout(() => state.voiceChannelId && openSheet("pt-voice-open"), 60);
}

function wireChatHeader() {
  $("pt-chat-title").onclick = () => openSheet("pt-chat-menu");
  $("pt-room-chip").onclick = backToRoom;
  document.querySelector(".chat-header .header-actions").addEventListener("click", e => {
    if (!$("app").classList.contains("pt-chat-menu") || !e.target.closest("button")) return;
    dropSheets();
  });
}

function fakeAnchor() {
  const rect = { left: 16, right: 16, top: window.innerHeight * 0.4, bottom: window.innerHeight * 0.4, width: 0, height: 0 };
  return { getBoundingClientRect: () => rect, closest: () => null };
}

function holdRow(title, run) {
  const spec = sheetRow(title);
  const row = el("button", "arow" + (spec.danger ? " danger" : ""));
  row.type = "button";
  if (spec.icon && StudioIcons.known(spec.icon)) row.insertAdjacentHTML("beforeend", StudioIcons.markup(spec.icon));
  row.appendChild(el("span", "l", spec.label));
  if (spec.hint) row.appendChild(el("small", "", spec.hint));
  row.onclick = () => closeSheets(run);
  return row;
}

function openHoldSheet(msg) {
  const bar = msg.querySelector(":scope > .msg-actions");
  const message = bar && messageOfActions.get(bar);
  if (!bar || !message) return false;
  const buttons = [...bar.querySelectorAll("button")];
  const rows = $("pt-hold-rows");
  rows.replaceChildren();
  for (const button of buttons) {
    if (button.title === "Add Reaction") continue;
    if (button.title === "More") {
      if (message.content) rows.appendChild(holdRow("Copy text", () => copyText(plainText(message.content), "Text copied")));
      rows.appendChild(holdRow("More", () => openMessageMenu(message, fakeAnchor())));
      continue;
    }
    if (button.title === "Remind Me") {
      rows.appendChild(holdRow("Remind Me", () => openRemindPop(message, fakeAnchor())));
      continue;
    }
    rows.appendChild(holdRow(button.title, () => button.click()));
  }
  const reactions = $("pt-hold-rx");
  reactions.replaceChildren();
  const canReact = buttons.some(button => button.title === "Add Reaction");
  reactions.classList.toggle("hidden", !canReact);
  if (canReact) {
    const mine = new Set((message.reactions || []).filter(r => state.user && r.users.includes(state.user.id)).map(r => r.emoji));
    for (const emoji of quickReactions(frequentEmoji(frequentEmojiStore(), key => !key.startsWith("<:")))) {
      const key = el("button", "key sq lg rx-key" + (mine.has(emoji) ? " is-down" : ""), emoji);
      key.type = "button";
      key.setAttribute("aria-label", `React with ${emoji}`);
      key.onclick = () => closeSheets(() => setReaction(message, emoji, !mine.has(emoji)));
      reactions.appendChild(key);
    }
    const more = studioNode("button", "key sq lg rx-key", '<i data-i="smile-plus"></i>');
    more.type = "button";
    more.setAttribute("aria-label", "More reactions");
    more.onclick = () => closeSheets(() => buttons.find(button => button.title === "Add Reaction").click());
    reactions.appendChild(more);
  }
  if (!rows.children.length && !canReact) return false;
  msg.classList.add("pt-held");
  openSheet("pt-hold-open");
  return true;
}

function wireHoldSheet() {
  document.addEventListener("contextmenu", e => {
    if (!studioSlim() || !e.target.closest) return;
    const msg = e.target.closest("#messages .msg");
    if (!msg || e.target.closest("a, input, textarea")) return;
    if (!openHoldSheet(msg)) return;
    e.preventDefault();
    e.stopPropagation();
  }, true);
}

function withChannelMenu(row, use) {
  row.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 0, clientY: 0 }));
  const menu = $("channel-menu");
  const result = menu.classList.contains("hidden") ? null : use(menu);
  closeFloating();
  return result;
}

function closeRowSwipe() {
  const open = studioPhone.rowOpen;
  if (!open) return;
  studioPhone.rowOpen = null;
  open.row.classList.remove("pt-open");
  open.row.style.translate = "";
  open.acts.remove();
}

function openRowSwipe(row) {
  if (studioPhone.rowOpen && studioPhone.rowOpen.row === row) return;
  closeRowSwipe();
  const muted = withChannelMenu(row, menu => !menu.querySelector("#chm-unmute").classList.contains("hidden"));
  if (muted === null) return;
  const acts = studioNode("div", "pt-acts", `<button type="button" class="key sq tap primary act read" aria-label="Mark read"><i data-i="check-check"></i></button><button type="button" class="key sq tap act mute" aria-label="${muted ? "Unmute" : "Mute"}"><i data-i="${muted ? "bell" : "bell-off"}"></i></button>`);
  acts.style.top = `${row.offsetTop}px`;
  acts.style.height = `${row.offsetHeight}px`;
  acts.querySelector(".read").onclick = () => {
    closeRowSwipe();
    withChannelMenu(row, menu => menu.querySelector("#chm-read").click());
  };
  acts.querySelector(".mute").onclick = () => {
    closeRowSwipe();
    withChannelMenu(row, menu => menu.querySelector(muted ? "#chm-unmute" : '[data-mute="forever"]').click());
  };
  row.before(acts);
  row.classList.add("pt-open");
  row.style.translate = `${SWIPE.rowOpen}px 0`;
  studioPhone.rowOpen = { row, acts };
}

const RAIL_UNLIT_MS = 200;
const RAIL_FADE_MS = 140;
const STUDIO_SHEET_NODES = "#pt-hold, #pt-pick, #pt-search, .ctx-menu, .composer-menu, .audio-pop, .soundboard-pop, .text-style-pop";

function sheetAtTop(sheet, from) {
  for (let node = from; node && node !== sheet.parentNode; node = node.parentNode) if (node.scrollTop > 0) return false;
  return true;
}

function closeSwipedSheet(sheet) {
  if (["pt-hold", "pt-pick", "pt-search"].includes(sheet.id)) closeSheets();
  else closeFloating();
  sheet.style.transition = "";
  sheet.style.translate = "";
}

function swipeTarget(e) {
  const target = e.target;
  if (!target.closest) return null;
  const sheet = target.closest(STUDIO_SHEET_NODES);
  if (sheet) return target.closest("input, textarea, select") || !sheetAtTop(sheet, target) ? null : { kind: "sheet", node: sheet, up: sheet.matches(".pt-tray:not(.pt-tray-full)") };
  if (target.closest("input, textarea, select, pre, code, video, audio, .md-table")) return null;
  const touch = e.touches[0];
  const pane = phonePane();
  if (profileOpen()) return edgeBackStarts(touch.clientX) && !sheetOpen() ? { kind: "back", node: $("pt-profile"), profile: true } : null;
  if (pane !== "list" && edgeBackStarts(touch.clientX) && !sheetOpen()) return { kind: "back", node: document.querySelector("main.main") };
  const msg = pane === "chat" && target.closest("#messages .msg");
  if (msg && msg.querySelector(':scope > .msg-actions button[title="Reply"]')) return { kind: "reply", node: msg };
  const row = pane === "list" && studioPhone.tab === "home" && target.closest("#channel-list .channel:not(.sidebar-link):not(.voice-channel):not(.dm-thread)");
  if (row) return { kind: "row", node: row, wasOpen: Boolean(studioPhone.rowOpen && studioPhone.rowOpen.row === row) };
  return null;
}

function paintSwipe(swipe, dx) {
  if (swipe.kind === "reply") {
    const node = swipe.node;
    const cue = $("pt-reply-cue");
    clearTimeout(studioPhone.railTimer);
    cue.classList.remove("out");
    node.classList.add("pt-lifted");
    if (cue.previousElementSibling !== node || !node.classList.contains("pt-railed")) {
      for (const other of document.querySelectorAll(".msg.pt-railed")) other.classList.remove("pt-railed", "pt-lifted");
      node.classList.add("pt-railed", "pt-lifted");
      node.after(cue);
      cue.style.left = `${node.offsetLeft}px`;
      cue.style.top = `${node.offsetTop}px`;
      cue.style.width = `${node.offsetWidth}px`;
      cue.style.height = `${node.offsetHeight}px`;
      cue.classList.add("on");
    }
    node.style.translate = `${replyPull(dx)}px 0`;
    cue.classList.toggle("ready", replyCommits(dx));
    if (replyCommits(dx)) cue.classList.add("used");
  } else if (swipe.kind === "row") {
    swipe.node.style.translate = `${rowPull(dx, swipe.wasOpen)}px 0`;
  } else if (swipe.kind === "back") {
    swipe.node.style.translate = `${Math.max(0, dx) * 0.35}px 0`;
  } else if (swipe.kind === "sheet") {
    swipe.node.style.translate = `0 ${sheetPull(dx)}px`;
  }
}

function stowReplyRail(node) {
  const cue = $("pt-reply-cue");
  node.classList.remove("pt-railed", "pt-lifted");
  cue.classList.remove("on", "ready", "out", "used");
  if (cue.parentNode !== $("app")) $("app").appendChild(cue);
}

function endSwipe(swipe, dx, cancelled) {
  const node = swipe.node;
  node.classList.remove("pt-dragging");
  if (swipe.kind === "reply") {
    node.style.translate = "";
    clearTimeout(studioPhone.railTimer);
    const cue = $("pt-reply-cue");
    const wait = ms => (studioPhone.calm ? 0 : ms);
    studioPhone.railTimer = setTimeout(() => {
      cue.classList.remove("ready");
      studioPhone.railTimer = setTimeout(() => {
        node.classList.remove("pt-lifted");
        cue.classList.add("out");
        studioPhone.railTimer = setTimeout(() => stowReplyRail(node), wait(RAIL_FADE_MS));
      }, wait(RAIL_FADE_MS));
    }, wait(RAIL_UNLIT_MS));
    if (!cancelled && replyCommits(dx)) {
      const reply = node.querySelector(':scope > .msg-actions button[title="Reply"]');
      setTimeout(() => reply.click(), wait(SWIPE.chipAfterMs));
    }
  } else if (swipe.kind === "row") {
    const pull = rowPull(dx, swipe.wasOpen);
    if (!cancelled && rowStaysOpen(pull, swipe.wasOpen)) {
      if (swipe.wasOpen) node.style.translate = `${SWIPE.rowOpen}px 0`;
      else {
        node.style.translate = "";
        openRowSwipe(node);
      }
    } else {
      node.style.translate = "";
      if (swipe.wasOpen) closeRowSwipe();
    }
  } else if (swipe.kind === "back") {
    node.style.translate = "";
    if (!cancelled && edgeBackCommits(dx)) {
      if (swipe.profile) leaveProfileEditor();
      else phoneStep("back");
    }
  } else if (swipe.kind === "sheet") {
    const full = node.classList.contains("pt-tray-full");
    if (!cancelled && (full ? sheetCloses(dx) : swipe.up && trayOpens(dx))) {
      node.style.translate = "";
      node.classList.toggle("pt-tray-full", !full);
      return;
    }
    if (cancelled || !sheetCloses(dx)) {
      node.style.translate = "";
      return;
    }
    node.style.transition = "translate 200ms var(--ease-out)";
    node.style.translate = "0 calc(100% + 48px)";
    setTimeout(() => closeSwipedSheet(node), studioPhone.calm ? 0 : 200);
  }
}

function wireSwipes() {
  let swipe = null;
  document.addEventListener("touchstart", e => {
    swipe = null;
    if (!studioSlim() || e.touches.length !== 1) return;
    if (studioPhone.rowOpen && !e.target.closest(".pt-acts") && e.target.closest(".channel") !== studioPhone.rowOpen.row) closeRowSwipe();
    const found = swipeTarget(e);
    if (!found) return;
    swipe = { ...found, x: e.touches[0].clientX, y: e.touches[0].clientY, axis: null, dx: 0, from: 0 };
  }, { passive: true });
  document.addEventListener("touchmove", e => {
    if (!swipe || !e.touches[0]) return;
    const dx = e.touches[0].clientX - swipe.x;
    const dy = e.touches[0].clientY - swipe.y;
    const along = swipe.kind === "sheet" ? "y" : "x";
    const pull = along === "y" ? dy : dx;
    if (!swipe.axis) {
      swipe.axis = swipeAxis(dx, dy);
      if ((swipe.axis && swipe.axis !== along) || (swipe.axis === along && pull < 0 && !swipe.wasOpen && !swipe.up)) {
        swipe = null;
        return;
      }
      if (swipe.axis === along) {
        swipe.from = swipe.wasOpen ? 0 : pull;
        swipe.node.classList.add("pt-dragging");
        studioPhone.swipedAt = Date.now();
      }
    }
    if (swipe.axis !== along) return;
    swipe.dx = pull - swipe.from;
    studioPhone.swipedAt = Date.now();
    if (e.cancelable) e.preventDefault();
    paintSwipe(swipe, swipe.dx);
  }, { passive: false });
  const finish = cancelled => () => {
    if (swipe && swipe.axis === (swipe.kind === "sheet" ? "y" : "x")) endSwipe(swipe, swipe.dx, cancelled);
    swipe = null;
  };
  document.addEventListener("touchend", finish(false), { passive: true });
  document.addEventListener("touchcancel", finish(true), { passive: true });
  document.addEventListener("click", e => {
    if (!studioSlim() || !e.isTrusted) return;
    const justSwiped = Date.now() - (studioPhone.swipedAt || 0) < 350;
    const openRow = studioPhone.rowOpen && e.target.closest && e.target.closest(".channel") === studioPhone.rowOpen.row;
    if (!justSwiped && !openRow) return;
    if (e.target.closest && e.target.closest(".pt-acts")) return;
    e.preventDefault();
    e.stopPropagation();
    if (openRow && !justSwiped) closeRowSwipe();
  }, true);
  $("channel-list").addEventListener("scroll", closeRowSwipe, { passive: true });
}

function roomVoice() {
  return {
    muted: Boolean(state.voice && state.voice.muted),
    deafened: Boolean(state.voice && state.voice.deafened),
    camera: Boolean(state.cameraOn),
    sounds: !$("soundboard-pop").classList.contains("hidden"),
    hand: $("voice-panel").classList.contains("in-audience") ? { raised: $("vp-hand").classList.contains("on") } : null,
    talk: !$("vp-talk").classList.contains("hidden"),
    talking: Boolean(state.talking),
  };
}

function roomServer() {
  const vs = state.voiceServer;
  if (!vs) return null;
  if (state.currentServer && state.currentServer.id === vs.id) return state.currentServer;
  return studioPhone.servers.find(server => server.id === vs.id) || null;
}

function roomOpen() {
  return $("app").classList.contains("pt-voice-open") && !$("phone-voice").classList.contains("hidden");
}

function syncRoomKeys() {
  const voice = roomVoice();
  const call = Boolean(state.voiceServer && state.voiceServer.call);
  for (const key of document.querySelectorAll("[data-room-key]")) {
    const id = key.dataset.roomKey;
    if (id === "leave") continue;
    const now = roomKeyState(id, voice);
    const resting = now.mode === "talk" && !now.held;
    key.classList.toggle("muted", resting || ((id === "mic" || id === "deafen") && !now.mode && now.down));
    key.classList.toggle("is-down", id === "sounds" && now.down);
    key.classList.toggle("is-off", Boolean(now.off));
    key.classList.toggle("is-on", Boolean(now.on) || (now.mode === "hand" && now.down));
    if (id === "mic") {
      key.classList.toggle("cream", now.mode !== "hand");
      key.classList.toggle("hand", now.mode === "hand");
      key.classList.toggle("talk", now.mode === "talk");
      const word = key.parentNode.querySelector(".lb");
      if (word) word.textContent = now.word || "Mic";
    }
    key.setAttribute("aria-label", now.label);
    key.setAttribute("aria-pressed", String(Boolean(now.down)));
    if (key.dataset.shown !== now.icon) {
      key.dataset.shown = now.icon;
      key.querySelector(".i").outerHTML = StudioIcons.markup(now.icon);
    }
    if (id === "camera") (key.closest(".k") || key).classList.toggle("gone", call);
  }
}

function holdTalk(down) {
  if (studioPhone.talkHeld === down) return;
  studioPhone.talkHeld = down;
  invoke("press_talk", { pressed: down }).catch(e => toast(String(e)));
}

function wireTalkKeys() {
  for (const key of document.querySelectorAll('[data-room-key="mic"]')) {
    key.addEventListener("pointerdown", e => {
      if (!key.classList.contains("talk")) return;
      key.setPointerCapture(e.pointerId);
      holdTalk(true);
    });
    for (const type of ["pointerup", "pointercancel", "lostpointercapture"]) key.addEventListener(type, () => holdTalk(false));
  }
}

function pressRoomKey(id) {
  const mode = id === "mic" ? roomKeyState(id, roomVoice()).mode : "";
  if (mode === "talk") return;
  if (mode === "hand") $("vp-hand").click();
  else if (id === "mic") $("mute-btn").click();
  else if (id === "deafen") $("deafen-btn").click();
  else if (id === "camera") $("vp-camera").click();
  else if (id === "sounds") {
    if (studioPhone.soundsWereOpen) closeFloating();
    else $("vp-soundboard").click();
  } else if (id === "leave") {
    if (hangUpTap(studioPhone.armed) === "leave") {
      disarmHangUp();
      disconnectVoice();
    } else armHangUp();
  }
}

function roomTile(tile) {
  const node = el("div", "ptile" + (tile.you ? " you" : "") + (tile.muted || tile.deafened ? " mut" : ""));
  if (tile.id) node.dataset.user = tile.id;
  if (tile.id && state.speaking.has(tile.id)) node.classList.add("speaking");
  node.appendChild(makeAvatar(tile.id || tile.name, tile.name, "avatar"));
  node.appendChild(el("b", "", tile.label));
  if (tile.muted || tile.deafened) node.appendChild(studioNode("span", "mo", `<i data-i="${tile.deafened ? "headphone-off" : "mic-off"}"></i>`));
  return node;
}

function roomStreamTile(stream) {
  const keys = stream.own
    ? `<span class="eye by${stream.viewers ? "" : " hidden"}"><i data-i="eye"></i><b>${stream.viewers}</b></span><span class="stopk" role="button" tabindex="0">Stop</span>`
    : `<span class="key primary go">Watch</span><span class="max"><i data-i="maximize"></i></span>${stream.watching ? '<span class="x" role="button" tabindex="0" aria-label="Stop watching"><i data-i="x"></i></span>' : ""}`;
  const tile = studioNode("div", "pt-stream theme-dark" + (stream.own ? " own" : stream.watching ? " w" : ""), `<span class="who"><span class="live">Live</span><span class="who-name"></span></span>${keys}`);
  tile.querySelector(".who-name").textContent = stream.title;
  tile.prepend(makeAvatar(stream.user, stream.name, "avatar"));
  if (stream.own) {
    tile.querySelector(".who-name").replaceChildren(el("span", "full", stream.title), el("span", "short", stream.short));
    const seen = tile.querySelector(".eye").cloneNode(true);
    seen.classList.remove("by");
    tile.querySelector(".who").appendChild(seen);
    tile.querySelector(".stopk").onclick = () => $("vp-stop-share").click();
    return tile;
  }
  tile.setAttribute("role", "button");
  tile.tabIndex = 0;
  tile.setAttribute("aria-label", `Watch ${stream.title}`);
  if (stream.watching) {
    const pic = el("canvas", "pic");
    pic.setAttribute("aria-hidden", "true");
    tile.prepend(pic);
    tile.querySelector(".x").onclick = e => {
      e.stopPropagation();
      stopWatching();
    };
  }
  tile.onclick = async () => {
    await watchScreen(stream.user, stream.name);
    if (!$("video-pane").classList.contains("hidden") && phonePane() === "chat") $("app").classList.add("pt-stream-full");
    syncStreamFull();
  };
  return tile;
}

function roomAudienceChip(person) {
  const chip = el("div", "pt-aud-chip" + (person.hand ? " hand" : ""));
  chip.appendChild(makeAvatar(person.id || person.name, person.name, "avatar"));
  chip.appendChild(el("b", "", person.label));
  if (person.hand) {
    const dot = el("i", "dot");
    dot.setAttribute("aria-label", "Hand raised");
    chip.appendChild(dot);
  }
  return chip;
}

function mirrorStream(now) {
  studioPhone.mirror = 0;
  const pic = document.querySelector("#pt-room-streams .pic");
  if (!pic || !roomOpen()) return;
  studioPhone.mirror = requestAnimationFrame(mirrorStream);
  if (now - (studioPhone.mirrored || 0) < MIRROR_FRAME_MS) return;
  const frame = $("video-frame");
  const box = pic.parentNode.getBoundingClientRect();
  const crop = frame.classList.contains("blank") ? null : coverCrop(frame.width, frame.height, box.width, box.height);
  if (!crop) return;
  studioPhone.mirrored = now;
  const ratio = Math.min(window.devicePixelRatio || 1, 2);
  const width = Math.round(box.width * ratio);
  const height = Math.round(box.height * ratio);
  if (pic.width !== width || pic.height !== height) {
    pic.width = width;
    pic.height = height;
  }
  pic.getContext("2d").drawImage(frame, crop.sx, crop.sy, crop.sw, crop.sh, 0, 0, width, height);
  pic.parentNode.classList.toggle("on", true);
}

function paintRoom() {
  studioPhone.roomQueued = 0;
  const vs = state.voiceServer;
  if (!$("pt-room") || !vs || !state.voiceChannelId) return;
  syncRoomKeys();
  paintWatch();
  if (!roomOpen()) return;
  const me = {
    userId: state.user && state.user.id,
    name: state.user && state.user.name,
    channelId: state.voiceChannelId,
    watching: state.watchingUser,
    ...roomVoice(),
  };
  const call = Boolean(vs.call);
  const server = call ? null : roomServer();
  const channel = server ? server.channels.find(found => found.id === state.voiceChannelId) : null;
  const everyone = call ? duoTiles(vs.name, me) : roomTiles(channel, me);
  const { tiles, audience } = roomStage(channel, everyone);
  const mine = everyone.find(tile => tile.you);
  const viewers = shareViewers($("vp-viewers").textContent, $("vp-viewers").classList.contains("hidden"));
  const streams = roomTileStreams(call ? [] : roomStreams(server, state.voiceChannelId, me), { userId: me.userId, name: mine ? mine.name : me.name, sharing: Boolean(state.sharing), viewers });
  const name = call ? vs.name : vs.channelName;
  const quality = $("vp-signal").dataset.quality || "";
  const seen = roomSignature(everyone, streams, [name, vs.name, quality, viewers, audience.map(person => `${person.id}${person.hand ? "!" : ""}`).join(",")].join("/"));
  if (seen === studioPhone.roomSeen) return;
  studioPhone.roomSeen = seen;
  $("pt-room-name").textContent = name;
  $("pt-room-live").classList.toggle("hidden", !streams.length);
  $("pt-room-sub").textContent = roomHeading({ serverName: vs.name, count: everyone.length, call });
  $("pt-room-sig").dataset.quality = quality;
  $("pt-room-sig").setAttribute("aria-label", `${studioDockQuality(quality).line}. Voice options`);
  $("pt-room-sig-word").textContent = signalWord(quality);
  $("pt-room-streams").replaceChildren(...streams.map(roomStreamTile));
  $("pt-room-people").replaceChildren(...tiles.map(roomTile));
  $("pt-room-aud").classList.toggle("hidden", !audience.length);
  $("pt-room-aud-label").textContent = audienceLabel(audience.length);
  $("pt-room-aud-chips").replaceChildren(...audience.map(roomAudienceChip));
  if (!studioPhone.mirror) studioPhone.mirror = requestAnimationFrame(mirrorStream);
}

function queueRoomPaint() {
  if (studioPhone.roomQueued) return;
  studioPhone.roomQueued = requestAnimationFrame(paintRoom);
}

function syncRoomMode() {
  const h = horizonParts();
  const open = roomOpen();
  if (h.bar.classList.contains("rm") === open) return;
  h.bar.classList.toggle("rm", open);
  disarmHangUp();
  closeVoiceOptions();
  if (!open) $("soundboard-pop").classList.toggle("hidden", true);
  if (open) {
    studioPhone.roomSeen = "";
    paintRoom();
    refreshLiveRooms(false);
  }
  if (studioPhone.calm) layHorizon();
  else {
    studioPhone.followUntil = performance.now() + 700;
    horizonSpring("slide", { x: studioPhone.centre, target: horizonTarget() });
  }
}

function showStreamTools(on) {
  clearTimeout(studioPhone.toolsTimer);
  $("app").classList.toggle("pt-stream-tools", on);
  if (on) studioPhone.toolsTimer = setTimeout(restStreamTools, STREAM_TOOLS_MS);
}

function restStreamTools() {
  showStreamTools(streamToolsStay($("stage-ink").dataset.tool));
}

function syncStreamFull() {
  const app = $("app");
  const showing = !$("video-pane").classList.contains("hidden");
  if (!showing && app.classList.contains("pt-stream-full")) app.classList.remove("pt-stream-full");
  if (showing && studioSlim() && !studioPhone.streamShown && !app.classList.contains("pt-stream-full")) app.classList.add("pt-stream-full");
  studioPhone.streamShown = showing;
  const full = app.classList.contains("pt-stream-full");
  if ($("video-pane").classList.contains("theme-dark") !== full) $("video-pane").classList.toggle("theme-dark", full);
  if (!full && app.classList.contains("pt-stream-tools")) showStreamTools(false);
  syncRoomKeys();
  if (full) paintWatch();
}

function paintWatchField() {
  const field = $("pt-watch-field");
  if (!field) return;
  const armed = studioPhone.armed;
  field.textContent = armed ? "Tap again to leave" : $("message-input").placeholder || "Message";
  field.setAttribute("aria-label", armed ? "Tap again to leave" : "Write a message");
}

function watchRows() {
  const rows = [];
  let author = "";
  let color = "";
  let face = null;
  for (const msg of document.querySelectorAll("#messages .msg")) {
    const name = msg.querySelector(".msg-author");
    if (name) {
      author = name.textContent;
      color = name.style.color || "";
      face = msg.querySelector(":scope > .avatar:not(.spacer)");
    }
    const content = msg.querySelector(".msg-content");
    rows.push({ id: msg.dataset.id, author, color, face, text: content ? content.textContent : "" });
  }
  return rows;
}

function paintWatch() {
  if (!$("app").classList.contains("pt-stream-full")) return;
  $("pt-watch-title").textContent = $("video-label").textContent;
  paintWatchField();
  const server = roomServer();
  const channel = server ? server.channels.find(found => found.id === state.voiceChannelId) : null;
  const people = roomFaces(channel ? roomPeople(channel) : [], state.user && state.user.id);
  $("pt-watch-sig").dataset.quality = $("vp-signal").dataset.quality || "good";
  const title = $("pt-watch-title");
  const place = $("pt-watch-room");
  const stack = $("pt-watch-people");
  for (const step of WATCH_STEPS) {
    const { one, shown, more } = watchStack(people, step, state.watchingUser);
    place.textContent = watchPlace(state.voiceServer, step.server);
    stack.classList.toggle("one", one);
    stack.replaceChildren(...shown.map(person => {
      const face = makeAvatar(person.id || person.name, person.name, "avatar");
      if (person.id) face.dataset.user = person.id;
      if (person.lead) face.classList.add("lead");
      if (person.id && state.speaking.has(person.id)) face.classList.add("speaking");
      return face;
    }));
    if (more) stack.appendChild(el("span", "stack-more", `+${more}`));
    if (title.scrollWidth <= title.clientWidth && place.scrollWidth <= place.clientWidth) break;
  }
  const said = streamBubbles(watchRows());
  const seen = said.map(row => `${row.id}:${row.text}`).join("|");
  if (seen === studioPhone.bubblesSeen) return;
  studioPhone.bubblesSeen = seen;
  $("pt-watch-bubbles").replaceChildren(...said.map(row => {
    const bubble = el("div", "bub");
    bubble.appendChild(row.face ? row.face.cloneNode(true) : makeAvatar(row.author, row.author, "avatar"));
    const words = el("div", "");
    const who = el("b", "", row.author);
    if (row.color) who.style.color = row.color;
    words.append(who, el("span", "", row.text));
    bubble.appendChild(words);
    return bubble;
  }));
}

function trayShown() {
  const pop = $("soundboard-pop");
  return pop.classList.contains("pt-tray") && !pop.classList.contains("hidden");
}

function trayPlace(target) {
  if (!target.closest) return "outside";
  if (target.closest("[data-room-key]")) return "key";
  if (target.closest(".soundboard-pop")) return "tray";
  return target.closest("#pt-room, .hzn") ? "room" : "outside";
}

function holdTrayStep(tray) {
  if (tray && trayNeedsStep(history.state)) {
    history.pushState({ ...history.state, ptTray: true }, "");
    studioPhone.trayStep = true;
  } else if (!tray && studioPhone.trayStep) {
    studioPhone.trayStep = false;
    if (history.state && history.state.ptTray) history.back();
  }
}

function decorateTray() {
  const pop = $("soundboard-pop");
  const tray = studioSlim() && roomOpen() && !pop.classList.contains("hidden");
  const was = pop.classList.contains("pt-tray");
  pop.classList.toggle("slab", tray);
  pop.classList.toggle("pt-tray", tray);
  $("pt-room").classList.toggle("tray", tray);
  if (tray !== was) holdTrayStep(tray);
  if (!tray) {
    pop.classList.toggle("pt-tray-full", false);
    if (studioPhone.trayHome && pop.parentNode !== studioPhone.trayHome.parent) studioPhone.trayHome.parent.insertBefore(pop, studioPhone.trayHome.next);
    return;
  }
  if (pop.parentNode !== $("pt-room")) {
    studioPhone.trayHome = { parent: pop.parentNode, next: pop.nextSibling };
    $("pt-room").appendChild(pop);
  }
  if (!pop.querySelector(".pt-tray-grab")) {
    const grab = el("button", "pt-tray-grab");
    grab.type = "button";
    grab.setAttribute("aria-label", "Show every sound");
    grab.onclick = () => pop.classList.toggle("pt-tray-full");
    pop.prepend(grab);
    pop.querySelector(".sb-head > span").after(el("small", "pt-tray-line", "The whole room hears these."));
    $("sb-grid").addEventListener("click", e => {
      const pad = e.target.closest(".sb-sound");
      if (!pad || pad.disabled || Date.now() < soundboardCooldown) return;
      pad.classList.toggle("playing", true);
      pad.addEventListener("animationend", () => pad.classList.toggle("playing", false), { once: true });
    }, true);
  }
  const grid = $("sb-grid");
  if (grid.querySelector(".pt-tray-add")) return;
  for (const pad of grid.querySelectorAll(".sb-sound")) pad.classList.add("key");
  const empty = grid.querySelector(".sb-empty");
  if (empty) empty.textContent = "No sounds from this server yet.";
  const add = studioNode("span", "key pt-tray-add", '<i data-i="plus"></i><span>Add sound</span>');
  add.setAttribute("role", "button");
  add.tabIndex = 0;
  add.classList.toggle("hidden", $("sb-add").classList.contains("hidden"));
  add.onclick = () => $("sb-add").click();
  grid.insertBefore(add, grid.querySelectorAll(".sb-section")[1] || null);
}

function chosenDevice(select, typed) {
  const option = select && select.selectedOptions[0];
  return typed || (option && option.value ? option.textContent : "Default");
}

function paintVoiceOptions() {
  const quality = $("vp-signal").dataset.quality || "good";
  const words = studioDockQuality(quality);
  const conn = $("pt-voice-conn");
  conn.dataset.quality = quality;
  conn.querySelector("b").textContent = words.line;
  conn.querySelector("small").textContent = words.sub;
  $("pt-voice-mic").textContent = chosenDevice(null, state.voice && state.voice.input_device);
  $("pt-voice-out").textContent = chosenDevice(null, state.voice && state.voice.output_device);
  $("pt-voice-cam").textContent = chosenDevice($("camera-device"), "");
  const body = document.body.classList;
  const share = $("pt-voice-share");
  share.classList.toggle("hidden", !canOfferShare({ mobileApp: body.contains("mobile-app"), canShare: body.contains("can-share-screen"), iPhone: /iPhone|iPod/.test(navigator.userAgent) }));
  share.querySelector(".l").textContent = state.sharing ? "Stop sharing" : "Share screen";
  const ink = $("pt-voice-ink");
  ink.classList.toggle("hidden", $("vp-ink-row").classList.contains("hidden"));
  ink.setAttribute("aria-checked", String(Boolean(state.sharing && state.shareInk)));
  const act = $("pt-voice-act");
  act.classList.toggle("hidden", $("vp-activities").classList.contains("hidden"));
  act.querySelector(".l").textContent = $("vp-activities").textContent.trim() || "Activities";
}

function openVoiceOptions() {
  paintVoiceOptions();
  $("pt-room").classList.add("opts");
  $("pt-room-sig").setAttribute("aria-expanded", "true");
}

function closeVoiceOptions() {
  $("pt-room").classList.remove("opts");
  $("pt-room-sig").setAttribute("aria-expanded", "false");
}

function wireRoom() {
  $("pt-room-down").onclick = () => closeSheets();
  $("pt-room-sig").onclick = () => {
    if ($("pt-room").classList.contains("opts")) closeVoiceOptions();
    else openVoiceOptions();
  };
  $("pt-voice-scrim").onclick = closeVoiceOptions;
  wireTalkKeys();
  $("pt-voice-sheet").onclick = e => {
    if (e.target.closest("#pt-voice-ink")) {
      setStreamInk(!state.shareInk).then(paintVoiceOptions);
      return;
    }
    const row = e.target.closest("[data-caret], [data-press]");
    if (!row) return;
    closeVoiceOptions();
    $(row.dataset.caret || row.dataset.press).click();
  };
  $("pt-watch").addEventListener("click", e => {
    const key = e.target.closest("[data-room-key]");
    if (key) {
      if (key.dataset.roomKey !== "leave") disarmHangUp();
      pressRoomKey(key.dataset.roomKey);
    } else if (e.target.closest("#pt-watch-down")) {
      disarmHangUp();
      backToRoom();
    } else if (e.target.closest("#pt-watch-field")) {
      const write = !studioPhone.armed;
      disarmHangUp();
      $("app").classList.remove("pt-stream-full");
      syncStreamFull();
      if (write) $("message-input").focus();
    }
  });
  $("stage-view").addEventListener("click", e => {
    if (!studioSlim() || !$("app").classList.contains("pt-stream-full")) return;
    if (e.target.closest(".stage-controls")) showStreamTools(true);
    else if (!e.target.closest(".pt-watch") && !$("stage-ink").dataset.tool) showStreamTools(!$("app").classList.contains("pt-stream-tools"));
  });
  $("stage-volume-range").addEventListener("input", () => studioSlim() && $("app").classList.contains("pt-stream-tools") && showStreamTools(true));
  $("stage-view").addEventListener("click", e => {
    if (!studioSlim() || !e.target.closest("#stage-fullscreen")) return;
    e.preventDefault();
    e.stopPropagation();
    $("app").classList.add("pt-stream-full");
    syncStreamFull();
  }, true);
}

function inviteShown() {
  return studioSlim() && !$("modal-backdrop").classList.contains("hidden") && !$("modal-invite-link").classList.contains("hidden");
}

function fillInvite() {
  const picture = $("invite-link-hero").style.backgroundImage;
  const wall = $("pt-invite-wall");
  wall.style.backgroundImage = picture;
  wall.classList.toggle("wash", !picture);
  $("pt-invite-tile").textContent = $("invite-link-icon").textContent;
  $("pt-invite-name").textContent = $("invite-link-name").textContent;
  $("pt-invite-from").textContent = $("invite-link-host").textContent;
  const warning = $("invite-link-warning");
  const parts = inviteWarningParts(warning.classList.contains("hidden") ? "" : warning.textContent);
  $("pt-invite-warn").classList.toggle("hidden", !parts.lead);
  $("pt-invite-warn-lead").textContent = parts.lead;
  $("pt-invite-warn-rest").textContent = parts.rest;
  $("pt-invite-error").classList.toggle("hidden", true);
}

function paintInviteKeys() {
  const busy = studioPhone.inviteBusy;
  $("pt-invite-join").disabled = busy;
  $("pt-invite-later").disabled = busy;
  $("pt-invite-join").textContent = busy ? "Joining\u2026" : "Join";
}

function syncInvite() {
  const open = inviteShown();
  const page = $("pt-invite");
  if (open && !page.classList.contains("on")) fillInvite();
  page.classList.toggle("on", open);
  const step = layerStep("ptInvite", open, studioPhone.inviteStep, history.state);
  if (step === "push") history.pushState({ ...(history.state || { pane: phonePane() }), ptInvite: true }, "");
  studioPhone.inviteStep = open;
  if (step === "back") history.back();
}

async function joinInviteHere() {
  const invite = pendingInvite;
  if (!invite || studioPhone.inviteBusy) return;
  studioPhone.inviteBusy = true;
  $("pt-invite-error").classList.toggle("hidden", true);
  paintInviteKeys();
  try {
    const summary = await invoke("join_server", { ticket: invite.ticket });
    pendingInvite = null;
    hideModals();
    await enterJoinedServer(summary);
  } catch (e) {
    $("pt-invite-error").textContent = String(e);
    $("pt-invite-error").classList.toggle("hidden", false);
  } finally {
    studioPhone.inviteBusy = false;
    paintInviteKeys();
  }
}

function wireInvite() {
  $("pt-invite-join").onclick = joinInviteHere;
  $("pt-invite-later").onclick = () => $("invite-link-cancel").click();
  const watch = node => new MutationObserver(syncInvite).observe(node, { attributes: true, attributeFilter: ["class"] });
  watch($("modal-backdrop"));
  watch($("modal-invite-link"));
  window.addEventListener("popstate", () => {
    if ((history.state && history.state.ptInvite) || !inviteShown()) return;
    if (studioPhone.inviteBusy) {
      history.pushState({ ...(history.state || { pane: phonePane() }), ptInvite: true }, "");
      return;
    }
    studioPhone.inviteStep = false;
    $("invite-link-cancel").click();
  });
}

function wireStudioPhone() {
  if (!studioLook() || !$("app") || typeof StudioIcons === "undefined") return;
  studioPhone.calm = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  buildStudioPhone();
  wireHorizon();
  wireYouPage();
  wireChatHeader();
  wireHomeHead();
  wireHoldSheet();
  wireSwipes();
  wireInvite();
  wireRoom();
  watchStudioPhone();
  $("app").dataset.tab = studioPhone.tab;
  paintHorizonTab(false);
  decorateRail();
  decorateRooms();
  syncHorizonVoice();
  syncHorizonBadge();
  syncPhoneTab();
  layHorizon();
}

wireStudioPhone();
