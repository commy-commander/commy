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

  const voice = webVoice(window);
  let started = null;
  host.startVoice = () => voice.start(frame => started && started.voice_frame(frame));
  host.stopVoice = () => voice.stop();
  host.playVoice = (peer, pcm) => voice.play(peer, pcm.slice());

  const client = import("./pkg/commy_web_client.js").then(async module => {
    await module.default();
    started = new module.WebClient(host, window.location.origin, webRelays(document));
    return started;
  });

  async function backend(cmd, args) {
    const reply = await (await client).invoke(cmd, JSON.stringify(args || {}));
    return JSON.parse(reply);
  }

  async function fileBytes(cmd, args) {
    const bytes = await (await client).file_bytes(cmd, JSON.stringify(args || {}));
    if (cmd === "download_attachment") return webSaveFile(document, URL, bytes, args && args.filename);
    return bytes.buffer;
  }

  async function attachPicked(args) {
    const file = await webPickFile(document, args && args.kind);
    if (!file) return null;
    if (file.size > WEB_MAX_SEND_BYTES) throw WEB_SEND_TOO_LARGE;
    const data = new Uint8Array(await file.arrayBuffer());
    return JSON.parse(await (await client).attach_dm(args.userId, file.name, data));
  }

  let unlocked = null;

  function unlockModal() {
    let modal = document.getElementById("modal-web-unlock");
    if (modal) return modal;
    modal = document.createElement("div");
    modal.className = "modal modal-signin hidden";
    modal.id = "modal-web-unlock";
    modal.innerHTML = `
      <img class="signin-logo" src="images/commy_v3_noback.png" alt="" />
      <h2>Open your servers here</h2>
      <p>Your messages stay between your devices. To open them in this browser, choose Allow in Commy on a device that already has them, or paste your recovery code.</p>
      <div class="signin-start" id="web-unlock-ask-row">
        <button class="btn-primary signin-go" id="web-unlock-ask">Ask again</button>
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
    modal.querySelector("#web-unlock-code-row").classList.toggle("hidden", noList || view.state === "opening");
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
  let askedDevices = false;

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
    if (view.state === "locked" && !askedDevices) {
      askedDevices = true;
      run("web_ask_devices");
    }
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
      window.location.assign(webSignInUrl(window.location.origin, window.location.pathname));
      return new Promise(() => {});
    }
    if (cmd === "account_sign_out") {
      await backend(cmd, args);
      window.location.reload();
      return new Promise(() => {});
    }
    if (WEB_BYTE_COMMANDS.includes(cmd)) return fileBytes(cmd, args);
    if (cmd === "attach_dm_file") return attachPicked(args);
    if (cmd === "get_user") {
      await untilUnlocked();
      if (typeof hideModals === "function") hideModals();
    }
    return backend(cmd, args);
  }

  const nudge = () => { if (unlocked) backend("web_nudge").catch(() => {}); };
  window.addEventListener("online", nudge);
  document.addEventListener("visibilitychange", () => { if (document.visibilityState === "visible") nudge(); });
  window.addEventListener("pagehide", () => { if (unlocked) backend("web_farewell").catch(() => {}); });
  window.addEventListener("pageshow", e => { if (e.persisted) nudge(); });

  listeners.set("web-unlock-changed", new Map([[0, e => e.payload.state !== "unlocked" && paintUnlock(e.payload)]]));
  listeners.set("web-servers-loaded", new Map([[0, () => { serversLoaded = true; }]]));

  window.__TAURI__ = {
    core: {
      invoke,
      convertFileSrc: (path, protocol) => webFileSrc(path, protocol),
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
  function setWebText(element, text) {
    if (!element) return;
    element.removeAttribute("data-i18n");
    element.textContent = text;
  }
  setWebText(document.getElementById("signin-go"), `Sign in on ${window.location.host}`);
  setWebText(document.querySelector("#signin-start .signin-note"), "Sign in or create your account, then come back to the web app.");
})();
