import { createAddSound } from "./add-sound.js";
import { findSound, serverOptions, settingsFor, soundGroups } from "./inspector-options.js";

const $ = id => document.getElementById(id);

let socket = null;
let context = "";
let action = "";
let settings = {};
let list = null;

function send(event, payload) {
  if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ event, action, context, payload }));
}

const addSound = createAddSound(payload => send("sendToPlugin", payload));

function option(value, label, disabled = false) {
  const node = document.createElement("option");
  node.value = value;
  node.textContent = label;
  node.disabled = disabled;
  return node;
}

function renderServers() {
  const select = $("server");
  select.replaceChildren(...serverOptions(list, settings.server).map(o => option(o.value, o.label, o.disabled)));
  select.value = settings.server ?? "";
  select.disabled = false;
}

function renderSounds() {
  const select = $("sound");
  const groups = soundGroups(list, settings.server ?? "");
  const nodes = [option("", "Choose a sound", true)];
  for (const group of groups) {
    const node = document.createElement("optgroup");
    node.label = group.label;
    node.append(...group.sounds.map(s => option(s.name, s.emoji ? `${s.emoji} ${s.label}` : s.label)));
    nodes.push(node);
  }
  select.replaceChildren(...nodes);
  select.value = findSound(groups, settings.sound) ? settings.sound : "";
  select.disabled = false;
}

function save(next) {
  settings = next;
  send("setSettings", settings);
}

function onSoundboards(message) {
  $("note").textContent = message.message || "";
  if (message.state !== "ready" || !message.list) return;
  list = message.list;
  renderServers();
  renderSounds();
  addSound.render(list, settings);
}

function onUploadResult(result) {
  if (result.ok && result.settings) settings = result.settings;
  addSound.finished(result);
}

$("server").addEventListener("change", e => {
  const server = e.target.value;
  const sound = findSound(soundGroups(list, server), settings.sound);
  save(settingsFor(server, sound));
  renderSounds();
});

$("sound").addEventListener("change", e => {
  const server = settings.server ?? "";
  save(settingsFor(server, findSound(soundGroups(list, server), e.target.value)));
});

window.connectElgatoStreamDeckSocket = (port, uuid, registerEvent, _info, actionInfo) => {
  const info = JSON.parse(actionInfo);
  context = uuid;
  action = info.action;
  settings = info.payload?.settings ?? {};
  socket = new WebSocket(`ws://127.0.0.1:${port}`);
  socket.onopen = () => {
    socket.send(JSON.stringify({ event: registerEvent, uuid }));
    send("sendToPlugin", { event: "refresh" });
  };
  socket.onmessage = e => {
    const message = JSON.parse(e.data);
    if (message.event === "sendToPropertyInspector" && message.payload?.event === "soundboards") onSoundboards(message.payload);
    if (message.event === "sendToPropertyInspector" && message.payload?.event === "upload-result") onUploadResult(message.payload);
    if (message.event === "didReceiveSettings") settings = message.payload?.settings ?? settings;
  };
};
