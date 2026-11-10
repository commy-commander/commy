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
  roomsAt: 0,
  roomsBusy: false,
  liveTimer: 0,
  badge: 0,
  calm: false,
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
</div>`;
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
  const bar = studioNode("nav", "hzn", horizonMarkup());
  bar.id = "horizon";
  bar.setAttribute("aria-label", "Places");
  app.append(title, live, you, scrim, bar);
  $("add-server-btn").dataset.ptLabel = "Add";
  $("discover-btn").dataset.ptLabel = "Discover";
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
    mic: bar.querySelector(".mic"),
    hang: bar.querySelector(".hang"),
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
  const offset = h.wrap.offsetLeft;
  studioPhone.centres = h.tabs.map(tab => offset + tab.offsetLeft + tab.offsetWidth / 2);
  h.bar.classList.toggle("tight", h.tabs[0].offsetWidth < 56);
}

function horizonTabIndex() {
  return Math.max(0, STUDIO_TABS.findIndex(tab => tab.id === studioPhone.tab));
}

function layHorizon() {
  if (!studioSlim() || !$("horizon").clientWidth) return;
  measureHorizon();
  studioPhone.centre = studioPhone.centres[horizonTabIndex()] ?? 0;
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
    const target = studioPhone.centres[horizonTabIndex()] ?? 0;
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
  else horizonSpring("slide", { x: studioPhone.centre, target: studioPhone.centres[horizonTabIndex()] ?? 0 });
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
  if (phonePane() !== "list") closeVoiceSheet();
}

function armHangUp() {
  const h = horizonParts();
  studioPhone.armed = true;
  h.bar.classList.add("armed");
  h.hang.classList.add("is-down");
  h.hang.setAttribute("aria-label", "Tap again to leave");
  clearTimeout(studioPhone.fuse);
  studioPhone.fuse = setTimeout(disarmHangUp, HANG_UP_FUSE_MS);
}

function disarmHangUp() {
  if (!studioPhone.armed) return;
  const h = horizonParts();
  studioPhone.armed = false;
  clearTimeout(studioPhone.fuse);
  h.bar.classList.remove("armed");
  h.hang.classList.remove("is-down");
  h.hang.setAttribute("aria-label", "Hang up");
}

function closeVoiceSheet() {
  const app = $("app");
  if (app.classList.contains("pt-voice-open")) app.classList.remove("pt-voice-open");
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
  h.open.setAttribute("aria-label", name ? `Voice controls for ${name}` : "Voice controls");
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

function personStack(people, className = "stack") {
  const { shown, more } = stackPeople(people);
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
    studioPhone.rooms = liveRooms(servers.filter(Boolean), { userId: state.user.id, channelId: state.voiceChannelId });
    studioPhone.roomsAt = Date.now();
  } finally {
    studioPhone.roomsBusy = false;
  }
  paintLivePage();
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
    if (e.target.closest(".mic")) {
      disarmHangUp();
      $("mute-btn").click();
      return;
    }
    if (e.target.closest(".open")) $("app").classList.toggle("pt-voice-open");
  });
  document.addEventListener("pointerdown", e => {
    if (studioPhone.armed && !h.hang.contains(e.target)) disarmHangUp();
  });
  document.addEventListener("keydown", e => {
    if (e.key === "Escape") {
      disarmHangUp();
      closeVoiceSheet();
    }
  });
  $("pt-scrim").onclick = closeVoiceSheet;
  $("channel-list").addEventListener("click", e => {
    if (!studioSlim() || e.target.closest("button")) return;
    if (!e.target.closest(".voice-channel.connected")) return;
    e.stopPropagation();
    e.preventDefault();
    $("app").classList.add("pt-voice-open");
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
  watch($("channel-list"), { childList: true }, () => {
    decorateRooms();
    refreshLiveRooms(false);
  });
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

function wireStudioPhone() {
  if (!studioLook() || !$("app") || typeof StudioIcons === "undefined") return;
  studioPhone.calm = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  buildStudioPhone();
  wireHorizon();
  wireYouPage();
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
