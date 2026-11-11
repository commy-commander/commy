function titlebarTint(server, tint) {
  return server && server.id && typeof tint === "function" ? tint(server.id) : "";
}

function titlebarIdle(focused, visible) {
  return !focused || visible === "hidden";
}

function studioTitlebar() {
  const bar = document.getElementById("titlebar");
  const icon = document.getElementById("titlebar-icon");
  const name = document.getElementById("titlebar-name");
  if (!bar || !icon || !name || bar.dataset.studioBar) return;
  bar.dataset.studioBar = "on";
  const paint = () => {
    const server = typeof plainServer === "function" ? plainServer() : null;
    const color = titlebarTint(server, typeof serverTint === "function" ? serverTint : null);
    if (color) icon.style.setProperty("--server-tint", color);
    else icon.style.removeProperty("--server-tint");
  };
  const front = focused => bar.classList.toggle("is-idle", titlebarIdle(focused, document.visibilityState));
  paint();
  const watch = new MutationObserver(paint);
  watch.observe(name, { childList: true, characterData: true, subtree: true });
  watch.observe(icon, { childList: true });
  front(document.hasFocus());
  const native = window.__TAURI__ && window.__TAURI__.window;
  if (native) {
    native.getCurrentWindow().onFocusChanged(({ payload }) => front(Boolean(payload))).catch(() => {});
  } else {
    window.addEventListener("focus", () => front(true));
    window.addEventListener("blur", () => front(false));
  }
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { titlebarTint, titlebarIdle };
}
