import {
  channelChoices,
  cleanAmount,
  deviceChoices,
  FIELDS,
  HINTS,
  kindOf,
  labelFor,
  personChoices,
  serverChoices,
  withSaved,
} from "./control-options.js";

const $ = id => document.getElementById(id);

let socket = null;
let context = "";
let action = "";
let kind = "";
let settings = {};
let directory = null;

function send(event, payload) {
  if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ event, action, context, payload }));
}

function save(changes) {
  settings = { ...settings, ...changes };
  send("setSettings", settings);
}

function fill(id, options, value, placeholder) {
  const select = $(id);
  const nodes = placeholder ? [new Option(placeholder, "", false, false)] : [];
  if (placeholder) nodes[0].disabled = true;
  for (const o of options) {
    const node = new Option(o.label, o.value);
    node.disabled = Boolean(o.disabled);
    nodes.push(node);
  }
  select.replaceChildren(...nodes);
  select.value = value ?? "";
  select.disabled = false;
}

function render() {
  const fields = FIELDS[kind] ?? [];
  for (const row of document.querySelectorAll("[data-field]")) row.hidden = !fields.includes(row.dataset.field);
  $("hint").textContent = HINTS[kind] ?? "";
  $("target").value = settings.target ?? "output";
  $("mode").value = settings.mode ?? "up";
  $("amount").value = String(settings.amount ?? 10);
  if (!directory) return;
  const servers = withSaved(serverChoices(directory), settings.server, kind === "server-stats" ? settings.label : undefined);
  fill("server", servers, settings.server, "Choose a server");
  fill("channel", withSaved(channelChoices(directory, settings.server, kind), settings.channel, settings.label), settings.channel, "Choose a channel");
  fill("user", withSaved(personChoices(directory), settings.user, settings.label), settings.user, "Choose a person");
  fill("device", withSaved(deviceChoices(directory, settings.target ?? "output"), settings.device, settings.label), settings.device ?? "");
}

function onDirectory(message) {
  $("note").textContent = message.message || "";
  if (message.state !== "ready" || !message.directory) return;
  directory = message.directory;
  render();
}

$("server").addEventListener("change", e => {
  const server = e.target.value;
  if (kind === "server-stats") save({ server, label: labelFor(serverChoices(directory), server) });
  else save({ server, channel: undefined, label: undefined });
  render();
});

$("channel").addEventListener("change", e => {
  const options = channelChoices(directory, settings.server, kind);
  save({ channel: e.target.value, label: labelFor(options, e.target.value)?.replace(/^#/, "") });
});

$("user").addEventListener("change", e => save({ user: e.target.value, label: labelFor(personChoices(directory), e.target.value) }));

$("target").addEventListener("change", e => {
  if (kind === "audio-device") save({ target: e.target.value, device: "", label: "System default" });
  else save({ target: e.target.value });
  render();
});

$("device").addEventListener("change", e => {
  save({ device: e.target.value, label: labelFor(deviceChoices(directory, settings.target ?? "output"), e.target.value) });
});

$("mode").addEventListener("change", e => save({ mode: e.target.value }));

$("amount").addEventListener("change", e => {
  const amount = cleanAmount(e.target.value);
  e.target.value = String(amount);
  save({ amount });
});

window.connectElgatoStreamDeckSocket = (port, uuid, registerEvent, _info, actionInfo) => {
  const info = JSON.parse(actionInfo);
  context = uuid;
  action = info.action;
  kind = kindOf(action);
  settings = info.payload?.settings ?? {};
  render();
  socket = new WebSocket(`ws://127.0.0.1:${port}`);
  socket.onopen = () => {
    socket.send(JSON.stringify({ event: registerEvent, uuid }));
    send("sendToPlugin", { event: "refresh" });
  };
  socket.onmessage = e => {
    const message = JSON.parse(e.data);
    if (message.event === "sendToPropertyInspector" && message.payload?.event === "directory") onDirectory(message.payload);
    if (message.event === "didReceiveSettings") settings = message.payload?.settings ?? settings;
  };
};
