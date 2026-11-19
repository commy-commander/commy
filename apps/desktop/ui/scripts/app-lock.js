const APP_LOCK_HINT_KEY = "commy.appLockOn";
const APP_LOCK_CHANGED_EVENT = "app-lock-changed";
const APP_LOCK_AFTER_CHOICES = [
  { secs: 0, label: "Immediately" },
  { secs: 60, label: "After 1 minute" },
  { secs: 300, label: "After 5 minutes" },
  { secs: 900, label: "After 15 minutes" },
  { secs: 3600, label: "After 1 hour" },
];
const APP_LOCK_WAYS = {
  face: { get button() { return t("ui.unlock-with-face-id"); }, setting: "Face ID" },
  touch: { get button() { return t("ui.unlock-with-touch-id"); }, setting: "Touch ID" },
  optic: { get button() { return t("ui.unlock-with-optic-id"); }, setting: "Optic ID" },
  fingerprint: { get button() { return t("ui.unlock-with-fingerprint"); }, get setting() { return t("ui.your-fingerprint"); } },
  biometrics: { get button() { return t("ui.unlock"); }, get setting() { return t("ui.your-face-or-fingerprint"); } },
  passcode: { get button() { return t("ui.unlock-with-passcode"); }, get setting() { return t("ui.your-phone-s-passcode"); } },
};

function appLockWay(unlockWith) {
  return APP_LOCK_WAYS[unlockWith] || APP_LOCK_WAYS.passcode;
}

function appLockButtonText(unlockWith) {
  return appLockWay(unlockWith).button;
}

function appLockSettingText(status) {
  if (!status || !status.phone_secure) return t("ui.set-a-passcode-on-your-phone-first");
  const way = appLockWay(status.unlock_with).setting;
  const fallback = status.unlock_with === "passcode" ? "" : ` ${t("ui.your-phone-s-passcode-works-too")}`;
  return `${t("ui.ask-for-way-to-open-commy", { way })}${fallback} ${t("ui.the-app-switcher-shows-a-blurred-screen")}`;
}

function appLockHint(status) {
  return status && status.supported && status.enabled ? "1" : "";
}

function appLockCoversOnLeave(status) {
  return Boolean(status && status.supported && status.enabled);
}

function appLockShouldAsk({ status, askOnLock, asking }) {
  return Boolean(status && status.locked && !status.asking && askOnLock && !asking);
}

function appLockCanceled(error) {
  return String(error || "").includes("canceled");
}

const AppLock = (() => {
  let status = null;
  let ask = null;
  let askOnLock = true;
  let asking = false;

  const byId = id => document.getElementById(id);

  function readHint() {
    try {
      return localStorage.getItem(APP_LOCK_HINT_KEY) === "1";
    } catch {
      return false;
    }
  }

  function writeHint(next) {
    try {
      if (appLockHint(next)) localStorage.setItem(APP_LOCK_HINT_KEY, "1");
      else localStorage.removeItem(APP_LOCK_HINT_KEY);
    } catch {}
  }

  function cover(on) {
    const screen = byId("app-lock");
    if (!screen) return;
    screen.classList.toggle("hidden", !on);
    document.documentElement.classList.toggle("app-locked", on);
  }

  function say(text) {
    const line = byId("app-lock-text");
    if (line) line.textContent = text || t("ui.unlock-to-see-your-messages");
  }

  function renderScreen() {
    const button = byId("app-lock-unlock");
    if (!button) return;
    button.textContent = appLockButtonText(status && status.unlock_with);
    button.disabled = asking || Boolean(status && status.asking);
  }

  function renderSettings() {
    const card = byId("app-lock-card");
    const toggle = byId("app-lock-toggle");
    const after = byId("app-lock-after");
    const afterRow = byId("app-lock-after-row");
    const sub = byId("app-lock-sub");
    if (!card || !toggle || !after) return;
    const supported = Boolean(status && status.supported);
    for (const part of document.querySelectorAll(".app-lock-part")) part.classList.toggle("hidden", !supported);
    if (!supported) return;
    toggle.checked = Boolean(status.enabled);
    toggle.disabled = !status.enabled && !status.phone_secure;
    after.value = String(status.after_secs);
    if (afterRow) afterRow.classList.toggle("hidden", !status.enabled);
    if (sub) sub.textContent = appLockSettingText(status);
  }

  function take(next) {
    status = next;
    writeHint(next);
    renderScreen();
    renderSettings();
    cover(Boolean(next && next.locked));
    if (next && next.notice && typeof toast === "function") toast(next.notice);
    return next;
  }

  async function unlock() {
    if (!ask || asking) return;
    asking = true;
    askOnLock = false;
    say();
    renderScreen();
    try {
      const next = await ask("app_lock_unlock");
      asking = false;
      take(next);
    } catch (e) {
      asking = false;
      renderScreen();
      say(String(e));
    }
  }

  async function refresh(command) {
    if (!ask) return;
    try {
      take(await ask(command));
    } catch {
      return;
    }
    if (appLockShouldAsk({ status, askOnLock, asking })) unlock();
  }

  function onVisibility() {
    if (!ask) return;
    if (document.visibilityState === "hidden") {
      if (asking) return;
      askOnLock = true;
      if (appLockCoversOnLeave(status)) cover(true);
      ask("app_lock_away").catch(() => {});
      return;
    }
    refresh("app_lock_back");
  }

  async function change() {
    const toggle = byId("app-lock-toggle");
    const after = byId("app-lock-after");
    if (!ask || !toggle || !after) return;
    toggle.disabled = true;
    asking = true;
    try {
      take(await ask("app_lock_set", { enabled: toggle.checked, afterSecs: Number(after.value) }));
    } catch (e) {
      if (!appLockCanceled(e) && typeof toast === "function") toast(String(e));
    }
    asking = false;
    renderSettings();
  }

  function wire() {
    const button = byId("app-lock-unlock");
    if (button) button.addEventListener("click", unlock);
    const toggle = byId("app-lock-toggle");
    if (toggle) toggle.addEventListener("change", change);
    const after = byId("app-lock-after");
    if (after) after.addEventListener("change", change);
    document.addEventListener("visibilitychange", onVisibility);
  }

  function start(invoker, listen) {
    ask = invoker;
    wire();
    if (listen) listen(APP_LOCK_CHANGED_EVENT, () => refresh("app_lock_status"));
    refresh("app_lock_status");
  }

  function coverFromHint() {
    if (readHint()) cover(true);
  }

  return { start, coverFromHint };
})();

if (typeof window !== "undefined" && typeof document !== "undefined" && window.__TAURI__) {
  AppLock.coverFromHint();
  const tauri = window.__TAURI__;
  const startLock = () => AppLock.start(tauri.core.invoke, tauri.event && tauri.event.listen);
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", startLock);
  else startLock();
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    APP_LOCK_HINT_KEY, APP_LOCK_AFTER_CHOICES, appLockButtonText, appLockSettingText,
    appLockHint, appLockCoversOnLeave, appLockShouldAsk, appLockCanceled,
  };
}
