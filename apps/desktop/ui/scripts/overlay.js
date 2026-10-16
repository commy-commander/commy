// In-game overlay window. Receives the voice roster and chat lines from
// the main window and speaking updates straight from the backend.

"use strict";

const tauri = window.__TAURI__;
const $ = id => document.getElementById(id);
const CHAT_LINES = 6;
const CHAT_FADE_MS = 30000;

const overlay = { members: [], speaking: new Set(), showChat: true };

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
  document.documentElement.style.setProperty("--opacity", String(config.opacity ?? 0.9));
  const corner = config.corner || "top-left";
  document.body.classList.toggle("bottom", corner.startsWith("bottom"));
  document.body.classList.toggle("right", corner.endsWith("right"));
  overlay.showChat = config.show_chat !== false;
  $("chat").classList.toggle("hidden", !overlay.showChat || !$("chat").childElementCount);
}

function renderRoster(roster) {
  overlay.members = (roster && roster.members) || [];
  $("voice").classList.toggle("hidden", !overlay.members.length);
  $("where").textContent = roster && roster.channel ? `${roster.channel} - ${roster.server}` : "";
  const list = $("members");
  list.replaceChildren();
  for (const member of overlay.members) {
    const row = el("div", "member" + (overlay.speaking.has(member.id) ? " speaking" : ""));
    row.dataset.user = member.id;
    const avatar = el("div", "avatar");
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
    const name = el("div", "name", member.name);
    if (member.color) name.style.color = `#${member.color}`;
    row.appendChild(name);
    list.appendChild(row);
  }
}

function setSpeaking(user, speaking) {
  if (speaking) overlay.speaking.add(user);
  else overlay.speaking.delete(user);
  for (const row of document.querySelectorAll(`.member[data-user="${CSS.escape(user)}"]`)) {
    row.classList.toggle("speaking", speaking);
  }
}

function addChatLine(line) {
  const chat = $("chat");
  const node = el("div", "line");
  const who = el("b", null, `${line.author}: `);
  if (line.color) who.style.color = `#${line.color}`;
  node.append(who, document.createTextNode(line.text));
  chat.appendChild(node);
  while (chat.childElementCount > CHAT_LINES) chat.firstElementChild.remove();
  chat.classList.toggle("hidden", !overlay.showChat);
  setTimeout(() => {
    node.classList.add("fading");
    setTimeout(() => {
      node.remove();
      if (!chat.childElementCount) chat.classList.add("hidden");
    }, 700);
  }, CHAT_FADE_MS);
}

async function start() {
  if (!tauri) return;
  await tauri.event.listen("overlay-config", e => applyConfig(e.payload));
  await tauri.event.listen("overlay-roster", e => renderRoster(e.payload));
  await tauri.event.listen("overlay-chat", e => addChatLine(e.payload));
  await tauri.event.listen("voice-speaking", e => setSpeaking(e.payload.user, e.payload.speaking));
  try {
    applyConfig(await tauri.core.invoke("get_overlay_settings"));
  } catch {
    /* defaults */
  }
  // Ask the main window for the current roster.
  await tauri.event.emitTo("main", "overlay-hello", null);
}

start();
