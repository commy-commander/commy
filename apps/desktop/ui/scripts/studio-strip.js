const STRIP_TILES = [
  { id: "messages", label: "Messages", icon: "message-circle", first: true },
  { id: "live", label: "Live", icon: "radio", first: true },
  { id: "you", label: "You", icon: "", first: false },
];
const STRIP_CHIP_GAP = 8;
const STRIP_ONLINE = "Online now";

function stripStacked(data) {
  return data.look === "studio" && data.fit === "slim" && data.stack === "on";
}

function stripChipOffsets(widths, gap = STRIP_CHIP_GAP) {
  let x = 0;
  return widths.map(width => {
    const at = x;
    x += width + gap;
    return at;
  });
}

function stripIsOnline(person) {
  const presence = (person && person.presence) || "offline";
  return presence !== "offline" && presence !== "invisible";
}

function stripPersonWord({ room, sharing, status, presence }) {
  if (sharing) return "Sharing a screen";
  if (room) return `In ${room}`;
  return status || presence || "";
}

function stripOnlineHead(count) {
  return `${STRIP_ONLINE} · ${count}`;
}

function stripPeople(server, meId) {
  if (!server) return [];
  const sharers = new Set((server.screen_shares || []).map(share => share.user));
  return (server.members || []).filter(member => member.id !== meId && stripIsOnline(member)).map(member => {
    const channel = (server.channels || []).find(found => member.voice_channel && found.id === member.voice_channel);
    const word = stripPersonWord({
      room: channel ? channelPlainName(channel) : "",
      sharing: sharers.has(member.id),
      status: backLine(member) || statusLine(member),
      presence: PRESENCE_LABELS[member.presence] || "",
    });
    return { member, word, inRoom: Boolean(channel) };
  });
}

function studioStrip() {
  const root = document.documentElement;
  const app = $("app");
  const bar = $("horizon");
  if (!bar && document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", studioStrip, { once: true });
    return;
  }
  if (!app || !bar || !$("server-list") || app.dataset.stripWired) return;
  app.dataset.stripWired = "on";
  let busy = false;
  let peopleSeen = "";

  const tileFor = id => app.querySelector(`.strip-tile[data-strip-tile="${id}"]`);

  const makeTile = spec => {
    const tile = el("button", `rail-btn rail-add strip-tile strip-${spec.id}`);
    tile.type = "button";
    tile.dataset.stripTile = spec.id;
    tile.dataset.ptLabel = spec.label;
    tile.setAttribute("aria-label", spec.label);
    tile.innerHTML = spec.icon ? studioIconMarkup(spec.icon) : "";
    if (spec.id === "messages") tile.appendChild(el("span", "strip-badge hidden"));
    if (spec.id === "live") tile.appendChild(el("span", "strip-dot hidden"));
    tile.onclick = () => setPhoneTab(spec.id);
    return tile;
  };

  const paintYou = () => {
    const tile = tileFor("you");
    if (!tile || !state.user) return;
    const key = `${state.user.id}/${state.user.name}/${$("me-avatar").innerHTML.length}`;
    if (tile.dataset.stripFace === key) return;
    tile.dataset.stripFace = key;
    tile.querySelector(".avatar")?.remove();
    tile.prepend(makeAvatar(state.user.id, state.user.name, "avatar"));
  };

  const paintMarks = () => {
    const badge = bar.querySelector('.tab[data-tab="messages"] .badge');
    const dot = bar.querySelector('.tab[data-tab="live"] .ld');
    const mine = tileFor("messages")?.querySelector(".strip-badge");
    const live = tileFor("live")?.querySelector(".strip-dot");
    if (mine && badge) {
      mine.textContent = badge.textContent;
      mine.classList.toggle("hidden", badge.classList.contains("hidden"));
    }
    if (live && dot) live.classList.toggle("hidden", dot.classList.contains("hidden"));
    for (const tile of app.querySelectorAll(".strip-tile")) tile.classList.toggle("strip-on", app.dataset.tab === tile.dataset.stripTile);
  };

  const placeTiles = () => {
    const list = $("server-list");
    for (const spec of STRIP_TILES) {
      const tile = tileFor(spec.id) || makeTile(spec);
      if (spec.first && tile.nextElementSibling !== (spec.id === "messages" ? tileFor("live") || list : list)) list.before(tile);
      if (!spec.first && $("discover-btn").nextElementSibling !== tile) $("discover-btn").after(tile);
    }
    const messages = tileFor("messages");
    const live = tileFor("live");
    if (messages.nextElementSibling !== live) live.before(messages);
    paintYou();
    paintMarks();
  };

  const placeChips = () => {
    const chips = [...$("channel-list").querySelectorAll(":scope > .sidebar-link")];
    const offsets = stripChipOffsets(chips.map(chip => chip.offsetWidth));
    chips.forEach((chip, i) => {
      const x = `${offsets[i]}px`;
      if (chip.style.getPropertyValue("--strip-x") !== x) chip.style.setProperty("--strip-x", x);
      chip.dataset.stripX = "on";
    });
  };

  const homeShown = () => app.dataset.tab === "home" && !app.classList.contains("dm-mode") && state.view === "server";

  const peopleNodes = () => [...$("channel-list").querySelectorAll(":scope > [data-strip-person]")];

  const placePeople = () => {
    const list = $("channel-list");
    const people = homeShown() ? stripPeople(state.currentServer, state.user && state.user.id) : [];
    const seen = people.map(p => `${p.member.id}:${p.word}:${p.member.name}`).join("|");
    let nodes = peopleNodes();
    if (seen !== peopleSeen || !nodes.length !== !people.length) {
      peopleSeen = seen;
      for (const node of nodes) node.remove();
      nodes = [];
      if (people.length) {
        const head = el("div", "strip-online-head", stripOnlineHead(people.length));
        head.dataset.stripPerson = "head";
        nodes.push(head);
        for (const person of people) {
          const row = el("button", "strip-person" + (person.inRoom ? " strip-in" : ""));
          row.type = "button";
          row.dataset.stripPerson = person.member.id;
          row.append(makeAvatar(person.member.id, person.member.name, "avatar"), el("span", "strip-name", person.member.name), el("small", "", person.word));
          row.onclick = e => {
            e.stopPropagation();
            openMemberCard(person.member, row);
          };
          nodes.push(row);
        }
      }
    }
    const tail = [...list.children].slice(-nodes.length);
    if (nodes.length && tail.some((node, i) => node !== nodes[i])) list.append(...nodes);
  };

  const undo = () => {
    for (const tile of app.querySelectorAll(".strip-tile")) tile.remove();
    for (const node of document.querySelectorAll("[data-strip-person]")) node.remove();
    peopleSeen = "";
    for (const chip of document.querySelectorAll("[data-strip-x]")) {
      chip.style.removeProperty("--strip-x");
      delete chip.dataset.stripX;
    }
    if (app.dataset.stripRoom) {
      delete app.dataset.stripRoom;
      app.classList.remove("pt-voice-open");
    }
  };

  const sync = () => {
    if (busy) return;
    busy = true;
    try {
      if (!stripStacked(root.dataset)) {
        undo();
        return;
      }
      if (app.dataset.pane && app.dataset.pane !== "list") app.dataset.pane = "list";
      const inVoice = !$("phone-voice").classList.contains("hidden");
      if (inVoice && !app.classList.contains("pt-voice-open")) {
        app.classList.add("pt-voice-open");
        app.dataset.stripRoom = "on";
      }
      if (!inVoice && app.dataset.stripRoom) {
        delete app.dataset.stripRoom;
        app.classList.remove("pt-voice-open");
      }
      placeTiles();
      placePeople();
      placeChips();
    } finally {
      busy = false;
    }
  };

  let queued = 0;
  const later = () => {
    if (queued) return;
    queued = requestAnimationFrame(() => {
      queued = 0;
      sync();
    });
  };
  const watch = (node, options) => node && new MutationObserver(later).observe(node, options);
  watch(root, { attributes: true, attributeFilter: ["data-fit", "data-stack", "data-look"] });
  watch(app, { attributes: true, attributeFilter: ["class", "data-pane", "data-tab"] });
  watch($("phone-voice"), { attributes: true, attributeFilter: ["class"] });
  watch($("channel-list"), { childList: true });
  watch($("member-list"), { childList: true });
  watch($("server-list"), { childList: true });
  watch($("me-avatar"), { childList: true, attributes: true, subtree: true });
  watch(bar.querySelector('.tab[data-tab="messages"] .badge'), { attributes: true, childList: true, characterData: true, subtree: true });
  watch(bar.querySelector('.tab[data-tab="live"] .ld'), { attributes: true });
  window.addEventListener("resize", later);
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(later);
  sync();
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { STRIP_TILES, STRIP_CHIP_GAP, STRIP_ONLINE, stripStacked, stripChipOffsets, stripIsOnline, stripPersonWord, stripOnlineHead };
}
