const STUDIO_TABS = [
  { id: "home", label: "Home", icon: "house" },
  { id: "messages", label: "Messages", icon: "message-circle" },
  { id: "live", label: "Live", icon: "radio" },
  { id: "you", label: "You", icon: "user" },
];
const STUDIO_TITLES = { messages: "Messages", live: "Live", you: "You" };
const STUDIO_LIVE_REFRESH_MS = 15000;

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
<div class="ks"><button type="button" class="key sq tap mic" aria-label="Mute"><i data-i="mic"></i><i class="led"></i></button><button type="button" class="key sq tap danger hang" aria-label="Hang up"><i data-i="phone"></i></button></div>
<button type="button" class="open" aria-label="Voice controls"><i data-i="bars"></i><span class="open-name"></span></button>
<div class="ask" role="status">Tap again to leave</div>
</div>
<div class="hzn-room">${ROOM_KEYS.map(roomKeyMarkup).join("")}<div class="ask" role="status">Tap again to leave</div></div>`;
}

function roomKeyMarkup(key) {
  const kind = key.id === "mic" ? "lg cream mic" : key.id === "leave" ? "tap danger hang" : "tap";
  return `<div class="k${key.id === "mic" ? " big" : ""}"><button type="button" class="key sq ${kind}" data-room-key="${key.id}" aria-label="${key.label}"><i data-i="${key.icon}"></i></button><span class="lb">${key.label}</span></div>`;
}

function roomMarkup() {
  return `<header class="pt-room-head">
<button type="button" class="pt-room-down" id="pt-room-down" aria-label="Back, and stay in the room"><i data-i="chevron-down"></i></button>
<div class="t"><b><span id="pt-room-name"></span><span class="live hidden" id="pt-room-live">Live</span></b><small id="pt-room-sub"></small></div>
<span class="sig" id="pt-room-sig"><i data-i="bars"></i><span id="pt-room-sig-word"></span></span>
<button type="button" class="key sq tap" id="pt-room-more" aria-label="Sound and camera options" aria-expanded="false"><i data-i="sliders-horizontal"></i></button>
</header>
<div class="pt-room-opts slab" id="pt-room-opts">
<button type="button" class="arow" data-caret="mute-caret"><i data-i="mic"></i><span class="l">Microphone options</span></button>
<button type="button" class="arow" data-caret="deafen-caret"><i data-i="headphones"></i><span class="l">Speaker options</span></button>
<button type="button" class="arow" data-caret="camera-caret"><i data-i="video"></i><span class="l">Camera options</span></button>
</div>
<div class="pt-room-body"><div class="pt-room-streams" id="pt-room-streams"></div><div class="pt-room-people" id="pt-room-people"></div></div>`;
}

function watchMarkup() {
  const key = id => {
    const spec = ROOM_KEYS.find(found => found.id === id);
    return `<button type="button" class="key sq ${id === "mic" ? "lg cream mic" : "tap"}" data-room-key="${id}" aria-label="${spec.label}"><i data-i="${spec.icon}"></i></button>`;
  };
  return `<div class="winfo">
<button type="button" class="key sq tap pt-watch-down" id="pt-watch-down" aria-label="Show the chat and keep watching"><i data-i="chevron-down"></i></button>
<div class="wcard"><div class="t"><span class="live">Live</span><span id="pt-watch-title"></span></div><div class="s"><i data-i="volume-2"></i><span id="pt-watch-room"></span></div></div>
<span class="stack" id="pt-watch-people"></span>
</div>
<div class="bubbles" id="pt-watch-bubbles"></div>
<div class="ctl">${["sounds", "camera", "deafen", "mic"].map(key).join("")}</div>
<div class="wbar"><button type="button" class="key sq tap danger hang" data-room-key="leave" aria-label="Hang up"><i data-i="phone"></i></button><button type="button" class="wfield" id="pt-watch-field"></button></div>`;
}

function youMarkup() {
  return `<div class="acct slab">
<button type="button" class="key sq tap edit" id="pt-edit" aria-label="Set status"><i data-i="pencil"></i></button>
<span class="avatar" id="pt-me-avatar"></span>
<div class="t"><b id="pt-me-name"></b><span id="pt-me-status"></span></div>
</div>
<div class="set slab">
<div class="sec lbl">Appearance</div>
<div class="srow"><i data-i="sun"></i><span class="l">Theme</span>
<div class="seg" role="group" aria-label="Theme"><button type="button" class="key tap" data-theme-choice="dark" aria-pressed="false"><i class="led"></i>Night</button><button type="button" class="key tap" data-theme-choice="light" aria-pressed="false"><i class="led"></i>Day</button></div>
</div>
<button type="button" class="srow" id="pt-glass" role="switch" aria-checked="false"><i data-i="layers"></i><span class="l">Glass<small>Off is lighter on the battery</small></span><i class="tog"></i></button>
<div class="sec lbl">More</div>
<button type="button" class="srow" id="pt-settings"><i data-i="settings"></i><span class="l">All settings</span><i data-i="chevron-right"></i></button>
</div>`;
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
  const scrim = el("div", "pt-scrim");
  scrim.id = "pt-scrim";
  const hold = studioNode("section", "pt-sheet pt-hold", '<div class="rx" id="pt-hold-rx"></div><div class="pt-sheet-rows" id="pt-hold-rows"></div>');
  hold.id = "pt-hold";
  hold.setAttribute("aria-label", "Message actions");
  const cue = studioNode("span", "pt-reply-cue", '<span class="rk"><i data-i="reply"></i></span>');
  cue.id = "pt-reply-cue";
  const bar = studioNode("nav", "hzn", horizonMarkup());
  bar.id = "horizon";
  bar.setAttribute("aria-label", "Places");
  const room = studioNode("section", "pt-room", roomMarkup());
  room.id = "pt-room";
  room.setAttribute("aria-label", "Voice room");
  app.append(title, live, you, scrim, hold, cue, room, bar);
  const watch = studioNode("div", "pt-watch", watchMarkup());
  watch.id = "pt-watch";
  $("stage-view").appendChild(watch);
  buildChatHeader();
  $("add-server-btn").dataset.ptLabel = "Add";
  $("discover-btn").dataset.ptLabel = "Discover";
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
  header.append(sub, title, chip);
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
  const d = horizonPath(width, studioPhone.centre, studioPhone.swell, pingAt, studioPhone.ping);
  for (const path of h.paths) path.setAttribute("d", d);
  h.grad.setAttribute("x2", width);
  h.hot.setAttribute("x1", (studioPhone.centre - 62).toFixed(1));
  h.hot.setAttribute("x2", (studioPhone.centre + 62).toFixed(1));
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
  if (tab === "messages" && state.view !== "dm") await enterDmMode();
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

const STUDIO_SHEETS = ["pt-voice-open", "pt-hold-open", "pt-chat-menu"];

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
    history.back();
    return;
  }
  if (then) then();
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
  const text = badgeText(count);
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
  for (const item of document.querySelectorAll("#server-list .rail-item")) {
    const button = item.querySelector(".server-icon");
    if (!button || item.querySelector(".pt-name")) continue;
    const name = el("span", "pt-name", button.title);
    if (button.classList.contains("active")) name.classList.add("on");
    item.appendChild(name);
  }
  for (const head of document.querySelectorAll("#server-list .folder-head")) {
    if (head.parentNode.querySelector(":scope > .pt-name")) continue;
    head.after(el("span", "pt-name", head.title));
  }
}

function personStack(people, className = "stack", max = STACK_MAX) {
  const { shown, more } = stackPeople(people, max);
  const stack = el("span", className);
  for (const person of shown) stack.appendChild(makeAvatar(person.id || person.name, person.name, "avatar"));
  if (more) stack.appendChild(el("span", "stack-more", `+${more}`));
  return stack;
}

function decorateRooms() {
  if (!studioLook()) return;
  const list = $("channel-list");
  for (const head of list.querySelectorAll(".section-head")) {
    head.classList.toggle("pt-community", Boolean(head.nextElementSibling && head.nextElementSibling.classList.contains("sidebar-link")));
  }
  const server = state.view === "server" ? state.currentServer : null;
  for (const row of list.querySelectorAll(".voice-channel[data-room]")) {
    row.querySelector(".pt-room-note")?.remove();
    row.querySelector(".stack")?.remove();
    const channel = server && server.channels.find(c => c.id === row.dataset.room);
    if (!channel) continue;
    const people = roomPeople(channel);
    const line = roomLine(people.length, state.voiceChannelId === channel.id);
    const sharing = liveRooms([server]).some(room => room.channelId === channel.id && room.streams.length > 0);
    row.classList.toggle("pt-busy", people.length > 0);
    const label = row.querySelector(".channel-label");
    label.querySelector(".live")?.remove();
    if (sharing) label.appendChild(el("span", "live", "Live"));
    if (line) {
      const note = el("span", "pt-room-note" + (state.voiceChannelId === channel.id ? " yes" : ""), line);
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
  $("pt-me-status").textContent = $("me-status").textContent;
  const theme = document.documentElement.dataset.theme === "light" ? "light" : "dark";
  for (const key of document.querySelectorAll("#pt-you [data-theme-choice]")) {
    const on = key.dataset.themeChoice === theme;
    key.classList.toggle("is-down", on);
    key.setAttribute("aria-pressed", String(on));
  }
  $("pt-glass").setAttribute("aria-checked", String(document.documentElement.dataset.fx !== "lite"));
}

function wireYouPage() {
  $("pt-edit").onclick = e => {
    e.stopPropagation();
    $("user-chip").click();
  };
  $("pt-settings").onclick = () => $("settings-btn").click();
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
  });
  document.addEventListener("keydown", e => {
    if (e.key === "Escape") {
      disarmHangUp();
      closeSheets();
    }
  });
  $("pt-scrim").onclick = () => closeSheets();
  window.addEventListener("popstate", () => {
    const back = history.state && history.state.ptSheet;
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
    refreshLiveRooms(false);
  });
  watch($("voice-panel"), { attributes: true, attributeFilter: ["class", "data-quality"], subtree: true, childList: true }, queueRoomPaint);
  watch($("soundboard-pop"), { attributes: true, attributeFilter: ["class"] }, syncRoomKeys);
  watch($("video-pane"), { attributes: true, attributeFilter: ["class"] }, syncStreamFull);
  watch($("video-label"), { childList: true, characterData: true, subtree: true }, paintWatch);
  watch($("messages"), { childList: true }, paintWatch);
  watch($("me-status"), { childList: true, characterData: true, subtree: true }, () => studioPhone.tab === "you" && paintYouPage());
  watch(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "data-fit", "data-fx"] }, () => {
    paintYouPage();
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
  const name = $("phone-voice-name").textContent;
  const inVoice = !$("phone-voice").classList.contains("hidden");
  $("pt-room-chip").classList.toggle("hidden", !inVoice);
  if ($("pt-room-chip-name").textContent !== name) $("pt-room-chip-name").textContent = name;
}

function wireChatHeader() {
  $("pt-chat-title").onclick = () => openSheet("pt-chat-menu");
  $("pt-room-chip").onclick = () => {
    if (phonePane() !== "list") phoneStep("back");
    setTimeout(() => state.voiceChannelId && openSheet("pt-voice-open"), 60);
  };
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
  const acts = studioNode("div", "pt-acts", `<button type="button" class="act read"><i data-i="check-check"></i>Mark read</button><button type="button" class="act mute"><i data-i="${muted ? "bell" : "bell-off"}"></i>${muted ? "Unmute" : "Mute"}</button>`);
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

function swipeTarget(e) {
  const target = e.target;
  if (!target.closest || target.closest("input, textarea, select, pre, code, video, audio, .md-table")) return null;
  const touch = e.touches[0];
  const pane = phonePane();
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
    if (cue.previousElementSibling !== node) {
      clearTimeout(studioPhone.railTimer);
      for (const other of document.querySelectorAll(".msg.pt-railed")) other.classList.remove("pt-railed");
      node.after(cue);
      cue.style.left = `${node.offsetLeft}px`;
      cue.style.top = `${node.offsetTop}px`;
      cue.style.width = `${node.offsetWidth}px`;
      cue.style.height = `${node.offsetHeight}px`;
      cue.classList.add("on");
      node.classList.add("pt-railed");
    }
    node.style.translate = `${replyPull(dx)}px 0`;
    cue.classList.toggle("ready", replyCommits(dx));
  } else if (swipe.kind === "row") {
    swipe.node.style.translate = `${rowPull(dx, swipe.wasOpen)}px 0`;
  } else if (swipe.kind === "back") {
    swipe.node.style.translate = `${Math.max(0, dx) * 0.35}px 0`;
  }
}

function stowReplyRail(node) {
  const cue = $("pt-reply-cue");
  node.classList.remove("pt-railed");
  cue.classList.remove("on", "ready");
  if (cue.parentNode !== $("app")) $("app").appendChild(cue);
}

function endSwipe(swipe, dx, cancelled) {
  const node = swipe.node;
  node.classList.remove("pt-dragging");
  if (swipe.kind === "reply") {
    node.style.translate = "";
    clearTimeout(studioPhone.railTimer);
    studioPhone.railTimer = setTimeout(() => stowReplyRail(node), studioPhone.calm ? 0 : 280);
    if (!cancelled && replyCommits(dx)) node.querySelector(':scope > .msg-actions button[title="Reply"]').click();
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
    if (!cancelled && edgeBackCommits(dx)) phoneStep("back");
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
    swipe = { ...found, x: e.touches[0].clientX, y: e.touches[0].clientY, axis: null, dx: 0 };
  }, { passive: true });
  document.addEventListener("touchmove", e => {
    if (!swipe || !e.touches[0]) return;
    const dx = e.touches[0].clientX - swipe.x;
    const dy = e.touches[0].clientY - swipe.y;
    if (!swipe.axis) {
      swipe.axis = swipeAxis(dx, dy);
      if (swipe.axis === "y" || (swipe.axis === "x" && dx < 0 && !swipe.wasOpen)) {
        swipe = null;
        return;
      }
      if (swipe.axis === "x") {
        swipe.node.classList.add("pt-dragging");
        studioPhone.swipedAt = Date.now();
      }
    }
    if (swipe.axis !== "x") return;
    swipe.dx = dx;
    studioPhone.swipedAt = Date.now();
    if (e.cancelable) e.preventDefault();
    paintSwipe(swipe, dx);
  }, { passive: false });
  const finish = cancelled => () => {
    if (swipe && swipe.axis === "x") endSwipe(swipe, swipe.dx, cancelled);
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
    key.classList.toggle("muted", (id === "mic" || id === "deafen") && now.down);
    key.classList.toggle("is-down", (id === "sounds" || id === "deafen") && now.down);
    key.classList.toggle("is-off", Boolean(now.off));
    key.setAttribute("aria-label", now.label);
    key.setAttribute("aria-pressed", String(Boolean(now.down)));
    if (key.dataset.shown !== now.icon) {
      key.dataset.shown = now.icon;
      key.querySelector(".i").outerHTML = StudioIcons.markup(now.icon);
    }
    if (id === "camera") (key.closest(".k") || key).classList.toggle("gone", call);
  }
}

function pressRoomKey(id) {
  if (id === "mic") $("mute-btn").click();
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
  if (tile.muted || tile.deafened) node.appendChild(studioNode("span", "mo", `<i data-i="${tile.deafened ? "headphones" : "mic-off"}"></i>`));
  return node;
}

function roomStreamTile(stream) {
  const tile = studioNode("button", "pt-stream theme-dark", `<span class="who"><span class="live">Live</span><span class="who-name"></span></span><span class="note"></span><span class="max"><i data-i="maximize"></i></span>`);
  tile.type = "button";
  tile.querySelector(".who-name").textContent = stream.title;
  tile.querySelector(".note").textContent = stream.watching ? "Watching now. Tap to go back to it" : "Tap to watch";
  tile.setAttribute("aria-label", `Watch ${stream.title}`);
  tile.prepend(makeAvatar(stream.user, stream.name, "avatar"));
  tile.onclick = async () => {
    await watchScreen(stream.user, stream.name);
    if (!$("video-pane").classList.contains("hidden") && phonePane() === "chat") $("app").classList.add("pt-stream-full");
    syncStreamFull();
  };
  return tile;
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
  const tiles = call ? duoTiles(vs.name, me) : roomTiles(channel, me);
  const streams = call ? [] : roomStreams(server, state.voiceChannelId, me);
  const name = call ? vs.name : vs.channelName;
  const quality = $("vp-signal").dataset.quality || "";
  const seen = roomSignature(tiles, streams, [name, vs.name, quality].join("/"));
  if (seen === studioPhone.roomSeen) return;
  studioPhone.roomSeen = seen;
  $("pt-room-name").textContent = name;
  $("pt-room-live").classList.toggle("hidden", !streams.length);
  $("pt-room-sub").textContent = roomHeading({ serverName: vs.name, count: tiles.length, call });
  $("pt-room-sig").dataset.quality = quality;
  $("pt-room-sig-word").textContent = signalWord(quality);
  $("pt-room-streams").replaceChildren(...streams.map(roomStreamTile));
  $("pt-room-people").replaceChildren(...tiles.map(roomTile));
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
  $("pt-room").classList.remove("opts");
  $("pt-room-more").setAttribute("aria-expanded", "false");
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

function syncStreamFull() {
  const app = $("app");
  const showing = !$("video-pane").classList.contains("hidden");
  if (!showing && app.classList.contains("pt-stream-full")) app.classList.remove("pt-stream-full");
  if (showing && studioSlim() && !studioPhone.streamShown && !app.classList.contains("pt-stream-full")) app.classList.add("pt-stream-full");
  studioPhone.streamShown = showing;
  const full = app.classList.contains("pt-stream-full");
  if ($("video-pane").classList.contains("theme-dark") !== full) $("video-pane").classList.toggle("theme-dark", full);
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
  $("pt-watch-room").textContent = watchPlace(state.voiceServer);
  paintWatchField();
  const server = roomServer();
  const channel = server ? server.channels.find(found => found.id === state.voiceChannelId) : null;
  const people = channel ? roomPeople(channel) : [];
  const stack = personStack(people, "stack", 3);
  for (const [i, face] of [...stack.querySelectorAll(".avatar")].entries()) {
    const person = people[i];
    if (!person || !person.id) continue;
    face.dataset.user = person.id;
    if (state.speaking.has(person.id)) face.classList.add("speaking");
  }
  $("pt-watch-people").replaceChildren(...stack.childNodes);
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

function wireRoom() {
  $("pt-room-down").onclick = () => closeSheets();
  $("pt-room-more").onclick = () => {
    const shown = $("pt-room").classList.toggle("opts");
    $("pt-room-more").setAttribute("aria-expanded", String(shown));
  };
  $("pt-room-opts").onclick = e => {
    const row = e.target.closest("[data-caret]");
    if (!row) return;
    $("pt-room").classList.remove("opts");
    $("pt-room-more").setAttribute("aria-expanded", "false");
    $(row.dataset.caret).click();
  };
  $("pt-watch").addEventListener("click", e => {
    const key = e.target.closest("[data-room-key]");
    if (key) {
      if (key.dataset.roomKey !== "leave") disarmHangUp();
      pressRoomKey(key.dataset.roomKey);
    } else if (e.target.closest("#pt-watch-down, #pt-watch-field")) {
      const write = Boolean(e.target.closest("#pt-watch-field")) && !studioPhone.armed;
      disarmHangUp();
      $("app").classList.remove("pt-stream-full");
      syncStreamFull();
      if (write) $("message-input").focus();
    }
  });
  $("stage-view").addEventListener("click", e => {
    if (!studioSlim() || !e.target.closest("#stage-fullscreen")) return;
    e.preventDefault();
    e.stopPropagation();
    $("app").classList.add("pt-stream-full");
    syncStreamFull();
  }, true);
}

function wireStudioPhone() {
  if (!studioLook() || !$("app") || typeof StudioIcons === "undefined") return;
  studioPhone.calm = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  buildStudioPhone();
  wireHorizon();
  wireYouPage();
  wireChatHeader();
  wireHoldSheet();
  wireSwipes();
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
