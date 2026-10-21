"use strict";

(() => {
  const listeners = new Map();
  let nextListener = 1;

  function emit(event, payloadJson) {
    const payload = payloadJson === undefined || payloadJson === "" ? null : JSON.parse(payloadJson);
    for (const callback of (listeners.get(event) || new Map()).values()) {
      try {
        callback({ event, payload });
      } catch (e) {
        console.error(e);
      }
    }
  }

  function storage() {
    try {
      return window.localStorage;
    } catch {
      return null;
    }
  }

  const host = {
    async request(method, path, body) {
      const init = { method, credentials: "same-origin", headers: {} };
      if (body !== undefined && body !== null) {
        init.headers["content-type"] = "application/json";
        init.body = body;
      }
      const res = await fetch(path, init);
      return webReplyLine(res.status, await res.text());
    },
    load(key) {
      const store = storage();
      return store ? store.getItem(key) : null;
    },
    save(key, value) {
      const store = storage();
      if (store) store.setItem(key, value);
    },
    remove(key) {
      const store = storage();
      if (store) store.removeItem(key);
    },
    emit,
  };

  const client = import("./pkg/commy_web_client.js").then(async module => {
    await module.default();
    return new module.WebClient(host, window.location.origin, webRelays(document));
  });

  async function backend(cmd, args) {
    const reply = await (await client).invoke(cmd, JSON.stringify(args || {}));
    return JSON.parse(reply);
  }

  let unlocked = null;

  function unlockModal() {
    let modal = document.getElementById("modal-web-unlock");
    if (modal) return modal;
    modal = document.createElement("div");
    modal.className = "modal modal-signin hidden";
    modal.id = "modal-web-unlock";
    modal.innerHTML = `
      <img class="signin-logo" src="images/commy_v2.png" alt="" />
      <h2>Open your servers here</h2>
      <p>Your messages stay between your devices. To open them in this browser, approve it from Commy on a device that already has them.</p>
      <div class="signin-start" id="web-unlock-ask-row">
        <button class="btn-primary signin-go" id="web-unlock-ask">Ask my devices</button>
      </div>
      <div class="signin-status hidden" id="web-unlock-wait"><span class="signin-spinner"></span><span id="web-unlock-wait-text"></span></div>
      <div class="signin-note" id="web-unlock-detail"></div>
      <div class="web-unlock-code" id="web-unlock-code-row">
        <input id="web-unlock-code" type="text" autocomplete="off" spellcheck="false" placeholder="Recovery code" />
        <button class="btn-secondary btn-small" id="web-unlock-code-use">Use code</button>
      </div>
      <div class="signin-error hidden" id="web-unlock-error"></div>`;
    document.getElementById("modal-signin").after(modal);
    modal.querySelector("#web-unlock-ask").onclick = () => run("web_ask_devices");
    modal.querySelector("#web-unlock-code-use").onclick = () => {
      const input = modal.querySelector("#web-unlock-code");
      run("entry_list_use_recovery_code", { code: input.value }).then(() => { input.value = ""; });
    };
    return modal;
  }

  function paintUnlock(view) {
    const modal = unlockModal();
    const asking = view.state === "asking" || view.state === "opening";
    const noList = view.state === "no_list";
    modal.querySelector("#web-unlock-ask-row").classList.toggle("hidden", asking || noList);
    modal.querySelector("#web-unlock-code-row").classList.toggle("hidden", noList || asking);
    modal.querySelector("#web-unlock-wait").classList.toggle("hidden", !asking);
    modal.querySelector("#web-unlock-wait-text").textContent = asking ? webUnlockText(view) : "";
    modal.querySelector("#web-unlock-detail").textContent = asking ? "" : webUnlockText(view);
  }

  function showUnlockError(message) {
    const box = unlockModal().querySelector("#web-unlock-error");
    box.textContent = message;
    box.classList.toggle("hidden", !message);
  }

  async function run(cmd, args) {
    showUnlockError("");
    try {
      settle(await backend(cmd, args));
    } catch (e) {
      showUnlockError(String(e));
    }
  }

  let resolveUnlocked = null;
  let serversLoaded = false;

  function whenServersLoad() {
    if (serversLoaded) return Promise.resolve();
    return new Promise(resolve => {
      const done = () => { serversLoaded = true; resolve(); };
      listeners.get("web-servers-loaded").set(-1, done);
      setTimeout(done, WEB_SERVER_WAIT_MS);
    });
  }

  function settle(view) {
    const opening = view.state === "unlocked";
    paintUnlock(opening ? { state: "opening", detail: "" } : view);
    if (opening) whenServersLoad().then(() => resolveUnlocked && resolveUnlocked());
    if (typeof showModal === "function") showModal("modal-web-unlock");
  }

  function untilUnlocked() {
    if (!unlocked) {
      unlocked = new Promise(resolve => { resolveUnlocked = resolve; });
      backend("web_unlock").then(settle, e => {
        settle({ state: "locked", detail: "" });
        showUnlockError(String(e));
      });
    }
    return unlocked;
  }

  async function invoke(cmd, args) {
    const fallback = webDefault(cmd);
    if (fallback.known) return fallback.value;
    if (cmd === "account_begin_sign_in") {
      window.location.assign(webSignInUrl(window.location.origin));
      return new Promise(() => {});
    }
    if (cmd === "get_user") {
      await untilUnlocked();
      if (typeof hideModals === "function") hideModals();
    }
    return backend(cmd, args);
  }

  listeners.set("web-unlock-changed", new Map([[0, e => e.payload.state !== "unlocked" && paintUnlock(e.payload)]]));
  listeners.set("web-servers-loaded", new Map([[0, () => { serversLoaded = true; }]]));

  window.__TAURI__ = {
    core: {
      invoke,
      convertFileSrc: path => path,
    },
    event: {
      listen(event, callback) {
        const id = nextListener++;
        if (!listeners.has(event)) listeners.set(event, new Map());
        listeners.get(event).set(id, callback);
        return Promise.resolve(() => listeners.get(event).delete(id));
      },
      emitTo: () => Promise.resolve(),
    },
  };
  document.documentElement.classList.add("web-app");
  const signInButton = document.getElementById("signin-go");
  if (signInButton) signInButton.textContent = `Sign in on ${window.location.host}`;
  const signInNote = document.querySelector("#signin-start .signin-note");
  if (signInNote) signInNote.textContent = "Sign in or create your account, then come back to the web app.";
})();
