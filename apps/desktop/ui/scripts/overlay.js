"use strict";

const tauri = window.__TAURI__;
const $ = id => document.getElementById(id);
const OVERLAY_LABEL = windowLabel();

const overlay = { members: [], speaking: new Set(), lingering: new Map(), showChat: true, showEveryone: false, picturesOnly: false, config: null, lines: [], testing: false };
const TEST_MEMBERS = [{ id: "overlay-test", name: "You, talking" }];
const tiles = new Map();
let noticeTimer = null;

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function windowLabel() {
  try {
    return tauri.window.getCurrentWindow().label;
  } catch {
    return "overlay";
  }
}

function initials(name) {
  return String(name || "?").split(/\s+/).filter(Boolean).slice(0, 2).map(w => w[0].toUpperCase()).join("") || "?";
}

function applyConfig(config) {
  document.documentElement.style.setProperty("--opacity", String(config.opacity ?? 0.92));
  const place = overlayPart(config, OVERLAY_LABEL);
  document.body.classList.toggle("bottom", place.bottom);
  document.body.classList.toggle("right", place.right);
  document.body.dataset.part = place.part;
  overlay.showChat = config.show_chat !== false;
  overlay.showEveryone = Boolean(config.show_everyone);
  overlay.picturesOnly = Boolean(config.pictures_only);
  document.body.classList.toggle("pictures", overlay.picturesOnly);
  document.body.classList.toggle("vertical", Boolean(config.stack_vertical));
  const notice = overlayMovedNotice(overlay.config, config);
  overlay.config = config;
  if (notice && place.part !== "chat") showNotice(notice);
  if (!overlay.showChat) {
    overlay.lines = [];
    $("chat").replaceChildren();
  }
  renderMembers();
}

function renderRoster(roster) {
  overlay.members = (roster && roster.members) || [];
  renderMembers();
}

function showNotice(text, ms = OVERLAY_NOTICE_MS) {
  const notice = $("notice");
  notice.textContent = text;
  notice.classList.remove("hidden");
  clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => notice.classList.add("hidden"), ms);
}

function runTest() {
  overlay.testing = true;
  showNotice("Overlay test", OVERLAY_TEST_MS);
  renderMembers();
  addChatLine({ author: "Commy", text: "Messages show for a few seconds, then go away.", force: true });
  setTimeout(() => {
    overlay.testing = false;
    renderMembers();
  }, OVERLAY_TEST_MS);
}

function fillTile(row, member) {
  const avatar = el("div", "avatar");
  avatar.style.setProperty("--tint", personTint(member.id));
  if (member.image) {
    const img = el("img");
    img.src = member.image;
    img.alt = "";
    avatar.appendChild(img);
  } else if (member.emoji) {
    avatar.classList.add("emoji");
    avatar.textContent = member.emoji;
  } else {
    avatar.textContent = initials(member.name);
  }
  row.replaceChildren(avatar);
  if (!overlay.picturesOnly) row.appendChild(el("div", "name", member.name));
}

function memberTile(member) {
  const key = overlayTileKey(member, overlay.picturesOnly);
  let row = tiles.get(member.id);
  if (!row) {
    row = el("div", "member");
    row.dataset.user = member.id;
    tiles.set(member.id, row);
  }
  if (row.dataset.key !== key) {
    row.dataset.key = key;
    fillTile(row, member);
  }
  return row;
}

function renderMembers() {
  const list = $("members");
  const lingering = new Set(overlay.lingering.keys());
  const speaking = overlay.testing ? new Set([...overlay.speaking, "overlay-test"]) : overlay.speaking;
  const members = overlay.testing ? [...TEST_MEMBERS, ...overlay.members] : overlay.members;
  const shown = overlayVisibleMembers({ members, speaking, lingering, showEveryone: overlay.showEveryone, picturesOnly: overlay.picturesOnly });
  const ids = new Set(shown.map(m => m.id));
  for (const [id, row] of tiles) {
    if (ids.has(id)) continue;
    row.remove();
    tiles.delete(id);
  }
  let cursor = list.firstElementChild;
  for (const member of shown) {
    const row = memberTile(member);
    row.classList.toggle("speaking", speaking.has(member.id));
    if (row === cursor) cursor = cursor.nextElementSibling;
    else list.insertBefore(row, cursor);
  }
}

function setSpeaking(user, speaking) {
  const pending = overlay.lingering.get(user);
  if (pending) clearTimeout(pending);
  overlay.lingering.delete(user);
  if (speaking) {
    overlay.speaking.add(user);
  } else {
    overlay.speaking.delete(user);
    overlay.lingering.set(user, setTimeout(() => {
      overlay.lingering.delete(user);
      renderMembers();
    }, OVERLAY_LINGER_MS));
  }
  renderMembers();
}

function resetOverlay() {
  for (const pending of overlay.lingering.values()) clearTimeout(pending);
  overlay.lingering.clear();
  overlay.speaking.clear();
  overlay.members = [];
  overlay.lines = [];
  $("chat").replaceChildren();
  renderMembers();
  tauri.event.emitTo("main", "overlay-hello", null).catch(() => {});
}

function addChatLine(line) {
  if (!overlay.showChat && !line.force) return;
  const chat = $("chat");
  const node = el("div", "line");
  const who = el("b", null, `${line.author}: `);
  if (line.color) who.style.color = `#${line.color}`;
  node.append(who, document.createTextNode(line.text));
  chat.appendChild(node);
  overlay.lines = overlayPushLine(overlay.lines, node);
  for (const old of [...chat.children]) {
    if (!overlay.lines.includes(old)) old.remove();
  }
  setTimeout(() => {
    node.classList.add("fading");
    setTimeout(() => {
      node.remove();
      overlay.lines = overlay.lines.filter(n => n !== node);
    }, 550);
  }, OVERLAY_CHAT_MS);
}

function osPrefers(query) {
  return typeof matchMedia === "function" && matchMedia(query).matches;
}

function applyOverlayAccessibility() {
  let raw = null;
  try { raw = localStorage.getItem(ACCESSIBILITY_KEY); } catch {}
  const prefs = readAccessibility(raw);
  const resolved = resolveAccessibility(prefs, { reducedMotion: osPrefers("(prefers-reduced-motion: reduce)"), dark: osPrefers("(prefers-color-scheme: dark)") });
  const html = document.documentElement;
  for (const [name, value] of Object.entries(accessibilityAttributes(resolved))) {
    if (value === null) html.removeAttribute(name);
    else html.setAttribute(name, value);
  }
  html.style.setProperty("--saturation", saturationCss(resolved));
}

async function start() {
  applyOverlayAccessibility();
  window.addEventListener("storage", e => {
    if (e.key === ACCESSIBILITY_KEY) applyOverlayAccessibility();
  });
  if (!tauri) return;
  const own = { target: { kind: "WebviewWindow", label: OVERLAY_LABEL } };
  await tauri.event.listen("overlay-config", e => applyConfig(e.payload), own);
  await tauri.event.listen("overlay-roster", e => renderRoster(e.payload), own);
  await tauri.event.listen("overlay-chat", e => addChatLine(e.payload), own);
  await tauri.event.listen("overlay-test", () => runTest(), own);
  await tauri.event.listen("overlay-reset", () => resetOverlay(), own);
  await tauri.event.listen("voice-speaking", e => setSpeaking(e.payload.user, e.payload.speaking));
  try {
    applyConfig(await tauri.core.invoke("get_overlay_settings"));
  } catch {}
  await tauri.event.emitTo("main", "overlay-hello", null);
}

if (!tauri && typeof location !== "undefined" && location.hash.startsWith("#preview")) {
  const opts = location.hash.slice(9).replace(/^=/, "").split(",");
  applyConfig({ corner: opts[0] || "top-left", show_everyone: opts.includes("everyone"), pictures_only: opts.includes("pictures"), stack_vertical: opts.includes("vertical"), show_chat: true, opacity: 0.92 });
  renderRoster({ members: [
    { id: "ferris", name: "ferris", emoji: "🦀" },
    { id: "nova", name: "nova" },
    { id: "rusty", name: "rusty" },
  ] });
  setSpeaking("ferris", true);
  setSpeaking("nova", true);
  addChatLine({ author: "nova", color: "9b7bff", text: "push to the left side, I'll hold mid" });
  addChatLine({ author: "ferris", text: "on it" });
}

start();
