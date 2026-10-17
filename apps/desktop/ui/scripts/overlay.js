// In-game overlay window. Receives the voice roster and chat lines from
// the main window and speaking updates straight from the backend. It
// draws only who is talking (or everyone, if asked) and chat lines that
// fade by themselves; the layout rules live in overlay-view.js.

"use strict";

const tauri = window.__TAURI__;
const $ = id => document.getElementById(id);

const overlay = { members: [], speaking: new Set(), lingering: new Map(), showChat: true, showEveryone: false, lines: [], testing: false };
const TEST_MEMBERS = [{ id: "overlay-test", name: "You, talking" }];
let noticeTimer = null;

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function initials(name) {
  return String(name || "?").split(/\s+/).filter(Boolean).slice(0, 2).map(w => w[0].toUpperCase()).join("") || "?";
}

function applyConfig(config) {
  document.documentElement.style.setProperty("--opacity", String(config.opacity ?? 0.92));
  const corner = overlayCorner(config.corner);
  document.body.classList.toggle("bottom", corner.bottom);
  document.body.classList.toggle("right", corner.right);
  overlay.showChat = config.show_chat !== false;
  overlay.showEveryone = Boolean(config.show_everyone);
  // Shows whenever the overlay appears or moves, so it is clear it is on and where.
  showNotice(`Overlay on · ${overlayCornerLabel(config.corner)}`);
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

// The Test overlay button: a sample speaker and message, then it clears.
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

function renderMembers() {
  const list = $("members");
  list.replaceChildren();
  const lingering = new Set(overlay.lingering.keys());
  const speaking = overlay.testing ? new Set([...overlay.speaking, "overlay-test"]) : overlay.speaking;
  const members = overlay.testing ? [...TEST_MEMBERS, ...overlay.members] : overlay.members;
  for (const member of overlayVisibleMembers({ members, speaking, lingering, showEveryone: overlay.showEveryone })) {
    const row = el("div", "member" + (speaking.has(member.id) ? " speaking" : ""));
    row.dataset.user = member.id;
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
    row.appendChild(avatar);
    row.appendChild(el("div", "name", member.name));
    list.appendChild(row);
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
    // Stay lit a moment so a short pause does not make the pill blink.
    overlay.lingering.set(user, setTimeout(() => {
      overlay.lingering.delete(user);
      renderMembers();
    }, OVERLAY_LINGER_MS));
  }
  renderMembers();
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

async function start() {
  if (!tauri) return;
  await tauri.event.listen("overlay-config", e => applyConfig(e.payload));
  await tauri.event.listen("overlay-roster", e => renderRoster(e.payload));
  await tauri.event.listen("overlay-chat", e => addChatLine(e.payload));
  await tauri.event.listen("overlay-test", () => runTest());
  await tauri.event.listen("voice-speaking", e => setSpeaking(e.payload.user, e.payload.speaking));
  try {
    applyConfig(await tauri.core.invoke("get_overlay_settings"));
  } catch {
    /* defaults */
  }
  // Ask the main window for the current roster.
  await tauri.event.emitTo("main", "overlay-hello", null);
}

// The page also runs outside the app for design previews: a hash like
// #preview=bottom-right,everyone fills it with sample people and lines.
if (!tauri && typeof location !== "undefined" && location.hash.startsWith("#preview")) {
  const opts = location.hash.slice(9).replace(/^=/, "").split(",");
  applyConfig({ corner: opts[0] || "top-left", show_everyone: opts.includes("everyone"), show_chat: true, opacity: 0.92 });
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
