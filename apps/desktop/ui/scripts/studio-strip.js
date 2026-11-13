const STRIP_TILES = [
  { id: "messages", label: "Messages", icon: "message-circle", first: true },
  { id: "live", label: "Live", icon: "radio", first: true },
  { id: "you", label: "You", icon: "", first: false },
];
const STRIP_CHIP_GAP = 8;
const STRIP_ONLINE = "Online now";
const STRIP_SHEET_CLOSE_PX = 80;
const STRIP_PANELS_KEY = "commy.tallStackPanels";
const STRIP_PANELS = [
  { id: "home", label: "Servers and channels", word: "appearance.stackPanel.home" },
  { id: "chat", label: "Chat", word: "appearance.stackPanel.chat" },
  { id: "room", label: "Voice room", word: "appearance.stackPanel.room" },
];
const STRIP_PANEL_ROWS = { home: "auto", chat: "minmax(0, 1fr)", room: "auto" };

function stripPanelsFrom(stored) {
  let saved = [];
  try {
    saved = JSON.parse(stored || "[]");
  } catch {
    saved = [];
  }
  const known = new Set(STRIP_PANELS.map(panel => panel.id));
  const seen = new Set();
  const panels = [];
  for (const entry of Array.isArray(saved) ? saved : []) {
    if (!entry || !known.has(entry.id) || seen.has(entry.id)) continue;
    seen.add(entry.id);
    panels.push({ id: entry.id, on: entry.on !== false });
  }
  for (const panel of STRIP_PANELS) if (!seen.has(panel.id)) panels.push({ id: panel.id, on: true });
  return panels.some(panel => panel.on) ? panels : panels.map(panel => ({ ...panel, on: true }));
}

function stripPanelMove(panels, id, step) {
  const from = panels.findIndex(panel => panel.id === id);
  const to = from + step;
  if (from < 0 || to < 0 || to >= panels.length) return panels;
  const moved = panels.slice();
  [moved[from], moved[to]] = [moved[to], moved[from]];
  return moved;
}

function stripPanelShow(panels, id, on) {
  const next = panels.map(panel => (panel.id === id ? { ...panel, on } : panel));
  return next.some(panel => panel.on) ? next : panels;
}

function stripPanelGrid(panels) {
  const shown = panels.filter(panel => panel.on);
  const chat = shown.some(panel => panel.id === "chat");
  const rows = ["96px"];
  const place = {};
  shown.forEach((panel, i) => {
    place[panel.id] = i + 2;
    rows.push(panel.id === "home" && !chat ? "minmax(0, 1fr)" : STRIP_PANEL_ROWS[panel.id]);
  });
  if (!rows.includes("minmax(0, 1fr)")) rows.push("minmax(0, 1fr)");
  return { rows: rows.join(" "), place, hidden: panels.filter(panel => !panel.on).map(panel => panel.id), chat };
}

function stripPanelsStored() {
  try {
    return window.localStorage.getItem(STRIP_PANELS_KEY);
  } catch {
    return null;
  }
}

function stripPanelsSave(panels) {
  try {
    window.localStorage.setItem(STRIP_PANELS_KEY, JSON.stringify(panels));
  } catch {
    return;
  }
  window.dispatchEvent(new Event("commy:stack-panels"));
}

function stripPanelWord(panel) {
  const word = typeof t === "function" ? t(panel.word) : panel.word;
  return word === panel.word ? panel.label : word;
}

function paintStripPanelSettings() {
  const box = $("stack-panels");
  if (!box) return;
  box.classList.toggle("hidden", !(typeof lookStackChosen === "function" && lookStackChosen()));
  const panels = stripPanelsFrom(stripPanelsStored());
  const list = $("stack-panels-list");
  list.replaceChildren();
  panels.forEach((panel, i) => {
    const spec = STRIP_PANELS.find(found => found.id === panel.id);
    const row = el("div", "stack-panel");
    row.dataset.panel = panel.id;
    const name = el("span", "stack-panel-name", stripPanelWord(spec));
    const up = el("button", "icon-btn stack-panel-move");
    up.type = "button";
    up.dataset.move = "up";
    up.setAttribute("aria-label", `Move ${spec.label} up`);
    up.innerHTML = studioIconMarkup("chevron-up");
    up.disabled = i === 0;
    up.onclick = () => {
      stripPanelsSave(stripPanelMove(panels, panel.id, -1));
      paintStripPanelSettings();
    };
    const down = el("button", "icon-btn stack-panel-move");
    down.type = "button";
    down.dataset.move = "down";
    down.setAttribute("aria-label", `Move ${spec.label} down`);
    down.innerHTML = studioIconMarkup("chevron-down");
    down.disabled = i === panels.length - 1;
    down.onclick = () => {
      stripPanelsSave(stripPanelMove(panels, panel.id, 1));
      paintStripPanelSettings();
    };
    const show = el("input", "switch");
    show.type = "checkbox";
    show.checked = panel.on;
    show.setAttribute("aria-label", `Show ${spec.label}`);
    show.onchange = () => {
      stripPanelsSave(stripPanelShow(panels, panel.id, show.checked));
      paintStripPanelSettings();
    };
    row.append(name, up, down, show);
    list.appendChild(row);
  });
}

function stripSheetDrag(dy) {
  return Math.max(0, Math.round(dy));
}

function stripSheetCloses(dy) {
  return dy >= STRIP_SHEET_CLOSE_PX;
}

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

  const peopleOpen = () => app.classList.contains("strip-people");

  const paintPeopleKey = () => {
    const key = app.querySelector(".strip-people-key");
    if (!key) return;
    const on = peopleOpen();
    key.classList.toggle("is-on", on);
    key.setAttribute("aria-pressed", String(on));
  };

  const openPeople = () => {
    if (peopleOpen()) return;
    app.classList.add("strip-people");
    history.pushState({ ...(history.state || {}), stripPeople: true }, "");
    paintPeopleKey();
  };

  const closePeople = how => {
    if (!peopleOpen()) return;
    app.classList.remove("strip-people");
    app.style.removeProperty("--strip-drag");
    paintPeopleKey();
    if (how === "history" || !(history.state && history.state.stripPeople)) return;
    if (how !== "leave") {
      history.back();
      return;
    }
    const entry = { ...history.state };
    delete entry.stripPeople;
    history.replaceState(entry, "");
  };

  const placeSheet = () => {
    const header = document.querySelector(".main .chat-header");
    if (header && !header.querySelector(":scope > .strip-people-key")) {
      const key = el("button", "key sq strip-people-key");
      key.type = "button";
      key.dataset.stripPeople = "key";
      key.setAttribute("aria-label", "Members");
      key.setAttribute("aria-pressed", "false");
      key.innerHTML = `${studioIconMarkup("users")}<i class="led"></i>`;
      key.onclick = e => {
        e.stopPropagation();
        if (peopleOpen()) closePeople();
        else openPeople();
      };
      header.appendChild(key);
    }
    if (!app.querySelector(":scope > .strip-scrim")) {
      const scrim = el("div", "strip-scrim");
      scrim.dataset.stripPeople = "scrim";
      scrim.onclick = () => closePeople();
      app.appendChild(scrim);
    }
    if (!app.querySelector(":scope > .strip-grab")) {
      const grab = el("button", "strip-grab");
      grab.type = "button";
      grab.dataset.stripPeople = "grab";
      grab.setAttribute("aria-label", "Close");
      grab.appendChild(el("i"));
      let from = null;
      grab.onpointerdown = e => {
        from = e.clientY;
        grab.setPointerCapture(e.pointerId);
      };
      grab.onpointermove = e => {
        if (from === null) return;
        app.style.setProperty("--strip-drag", `${stripSheetDrag(e.clientY - from)}px`);
      };
      grab.onpointerup = e => {
        if (from === null) return;
        const dy = e.clientY - from;
        from = null;
        app.style.removeProperty("--strip-drag");
        if (stripSheetCloses(dy)) closePeople();
      };
      grab.onpointercancel = () => {
        from = null;
        app.style.removeProperty("--strip-drag");
      };
      grab.onclick = e => {
        if (e.detail === 0) closePeople();
      };
      app.appendChild(grab);
    }
    paintPeopleKey();
  };

  const placePanels = () => {
    const grid = stripPanelGrid(stripPanelsFrom(stripPanelsStored()));
    const rows = { "--st-rows": grid.rows, "--st-home-row": grid.place.home, "--st-chat-row": grid.place.chat, "--st-room-row": grid.place.room };
    for (const [name, value] of Object.entries(rows)) {
      const want = value === undefined ? "" : String(value);
      if (app.style.getPropertyValue(name) !== want) {
        if (want) app.style.setProperty(name, want);
        else app.style.removeProperty(name);
      }
    }
    for (const panel of STRIP_PANELS) {
      const off = grid.hidden.includes(panel.id);
      if (app.classList.contains(`strip-hide-${panel.id}`) !== off) app.classList.toggle(`strip-hide-${panel.id}`, off);
    }
    if (app.classList.contains("strip-no-chat") === grid.chat) app.classList.toggle("strip-no-chat", !grid.chat);
  };

  const undoPanels = () => {
    for (const name of ["--st-rows", "--st-home-row", "--st-chat-row", "--st-room-row"]) app.style.removeProperty(name);
    for (const panel of STRIP_PANELS) app.classList.remove(`strip-hide-${panel.id}`);
    app.classList.remove("strip-no-chat");
  };

  const undo = () => {
    undoPanels();
    if (peopleOpen()) closePeople("leave");
    for (const node of document.querySelectorAll("[data-strip-people]")) node.remove();
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
      placePanels();
      placeTiles();
      placePeople();
      placeChips();
      placeSheet();
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
  watch(document.querySelector(".main .chat-header"), { childList: true });
  window.addEventListener("resize", later);
  window.addEventListener("commy:stack-panels", later);
  window.addEventListener("storage", e => {
    if (e.key === STRIP_PANELS_KEY) later();
  });
  window.addEventListener("popstate", () => {
    if (peopleOpen() && !(history.state && history.state.stripPeople)) closePeople("history");
  });
  document.addEventListener("keydown", e => {
    if (e.key !== "Escape" || !peopleOpen() || !$("member-card").classList.contains("hidden")) return;
    e.stopPropagation();
    closePeople();
  }, true);
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(later);
  sync();
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { STRIP_PANELS_KEY, STRIP_PANELS, stripPanelsFrom, stripPanelMove, stripPanelShow, stripPanelGrid, STRIP_TILES, STRIP_CHIP_GAP, STRIP_ONLINE, STRIP_SHEET_CLOSE_PX, stripSheetDrag, stripSheetCloses, stripStacked, stripChipOffsets, stripIsOnline, stripPersonWord, stripOnlineHead };
}
