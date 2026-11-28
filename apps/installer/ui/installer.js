"use strict";

const tauri = window.__TAURI__;
const invoke = (command, args) => tauri.core.invoke(command, args);
const appWindow = tauri.window.getCurrentWindow();

const ARROW = '<svg viewBox="0 0 12 12" aria-hidden="true"><path d="M4.5 2.5L8 6l-3.5 3.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>';
const INSTALL_STEPS = ["Putting Commy in place", "Adding the Start menu shortcut", "Setting up updates"];
const INSTALL_GUESS_MS = 9000;

const el = (id) => document.getElementById(id);
const title = el("title");
const sub = el("sub");
const keys = el("keys");
const meter = el("meter");
const track = el("track");
const fill = el("fill");
const pct = el("pct");
const extra = el("extra");

const choices = { desktopShortcut: true, startWithComputer: true };
let release = null;
let installTimer = 0;
let downloadStartedAt = 0;

const stillMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
const star = startStar();

function startStar() {
  try {
    const started = CommyStar.start(el("sky"), { still: stillMotion });
    if (started) return started;
  } catch (error) {
    console.warn("the star could not be drawn", error);
  }
  el("sky").hidden = true;
  el("still").hidden = false;
  return { set() {} };
}

function key(label, { main = false, onClick } = {}) {
  const button = document.createElement("button");
  button.className = main ? "key main" : "key";
  button.innerHTML = `<span></span>${main ? ARROW : ""}`;
  button.firstChild.textContent = label;
  button.addEventListener("click", onClick);
  return button;
}

function switchFor(label, choice) {
  const button = document.createElement("button");
  button.className = "switch";
  button.setAttribute("role", "switch");
  button.setAttribute("aria-checked", String(choices[choice]));
  button.innerHTML = "<i></i><span></span>";
  button.lastChild.textContent = label;
  button.addEventListener("click", () => {
    choices[choice] = !choices[choice];
    button.setAttribute("aria-checked", String(choices[choice]));
  });
  return button;
}

function show({ heading, line, buttons = [], progress = null, busy = false, below = [] }) {
  title.textContent = heading;
  sub.textContent = line;
  keys.replaceChildren(...buttons);
  meter.hidden = progress === null && !busy;
  track.classList.toggle("busy", busy);
  if (progress !== null) setProgress(progress);
  if (busy) pct.textContent = "";
  extra.replaceChildren(...below);
  extra.hidden = below.length === 0;
}

function setProgress(share) {
  const clamped = Math.max(0, Math.min(1, share));
  fill.style.width = `${(clamped * 100).toFixed(1)}%`;
  pct.textContent = `${Math.round(clamped * 100)}%`;
}

function megabytes(bytes) {
  return (bytes / 1_000_000).toFixed(1);
}

function secondsLeft(received, total, startedAt) {
  const elapsed = (performance.now() - startedAt) / 1000;
  if (received <= 0 || elapsed < 1) return null;
  return Math.max(1, Math.ceil(((total - received) / received) * elapsed));
}

function welcome() {
  clearInterval(installTimer);
  star.set({ hue: 0, energy: 0.9 });
  const line = release
    ? `Version ${release.version} for Windows${release.bytes ? `, ${megabytes(release.bytes)} MB` : ""}`
    : "Finding the newest version";
  show({ heading: "Install Commy", line, buttons: [key("Options", { onClick: options }), key("Install", { main: true, onClick: install })] });
}

function options() {
  show({
    heading: "Options",
    line: "Commy installs for you only, so it never asks for an administrator.",
    buttons: [key("Back", { onClick: welcome }), key("Install", { main: true, onClick: install })],
    below: [switchFor("Add a desktop shortcut", "desktopShortcut")],
  });
}

function install() {
  star.set({ hue: 0, energy: 1.9 });
  show({ heading: "Downloading", line: "Starting the download", buttons: [key("Cancel", { onClick: cancel })], progress: 0 });
  downloadStartedAt = performance.now();
  invoke("start_install", { desktopShortcut: choices.desktopShortcut });
}

function cancel() {
  invoke("cancel_install");
}

function downloading({ received, total }) {
  if (!total) {
    sub.textContent = `${megabytes(received)} MB so far`;
    return;
  }
  const share = received / total;
  const left = secondsLeft(received, total, downloadStartedAt);
  sub.textContent = `${megabytes(received)} of ${megabytes(total)} MB${left ? `, about ${left} ${left === 1 ? "second" : "seconds"} left` : ""}`;
  setProgress(share);
  star.set({ hue: share * 0.6, energy: 1.9 });
}

function checking() {
  star.set({ hue: 0.65, energy: 1.3 });
  show({ heading: "Checking the download", line: "Making sure it is signed by Commy and nothing changed on the way", buttons: [key("Cancel", { onClick: cancel })], busy: true });
}

function closing() {
  star.set({ hue: 0.65, energy: 1.2 });
  show({ heading: "Closing Commy", line: "Commy is open, so Setup closes it before putting the new version in place", busy: true });
}

function installing() {
  star.set({ hue: 0.65, energy: 1.4 });
  show({ heading: "Installing", line: INSTALL_STEPS[0], progress: 0 });
  const startedAt = performance.now();
  clearInterval(installTimer);
  installTimer = setInterval(() => {
    const share = 1 - Math.exp(-(performance.now() - startedAt) / (INSTALL_GUESS_MS / 2.5));
    const shown = share * 0.95;
    setProgress(shown);
    sub.textContent = INSTALL_STEPS[Math.min(INSTALL_STEPS.length - 1, Math.floor(shown * INSTALL_STEPS.length))];
    star.set({ hue: 0.65 + shown * 0.35 });
  }, 120);
}

function ready() {
  clearInterval(installTimer);
  star.set({ hue: 1, energy: 0.7 });
  show({
    heading: "Commy is ready",
    line: "You will find it in the Start menu too.",
    buttons: [key("Open Commy", { main: true, onClick: openCommy })],
    below: [switchFor("Start Commy when I sign in", "startWithComputer")],
  });
}

function failed(message) {
  clearInterval(installTimer);
  star.set({ hue: 0, energy: 0.6 });
  show({ heading: "Commy did not install", line: message, buttons: [key("Try again", { main: true, onClick: install })] });
}

async function openCommy() {
  try {
    await invoke("open_commy", { startWithComputer: choices.startWithComputer });
  } catch (message) {
    failed(String(message));
  }
}

const ON_PROGRESS = {
  download: downloading,
  check: checking,
  closing,
  install: installing,
  done: ready,
  cancelled: welcome,
  failed: ({ message }) => failed(message),
};

tauri.event.listen("install-progress", ({ payload }) => ON_PROGRESS[payload.step]?.(payload));

el("minimise").addEventListener("click", () => appWindow.minimize());
el("close").addEventListener("click", async () => {
  await invoke("cancel_install");
  appWindow.close();
});

async function findRelease() {
  try {
    release = await invoke("release_info");
    choices.startWithComputer = release.starts_with_computer;
    if (title.textContent === "Install Commy") welcome();
  } catch (message) {
    if (title.textContent === "Install Commy") sub.textContent = String(message);
  }
}

welcome();
findRelease();
