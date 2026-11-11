"use strict";

const MINORS_EDITABLE = "a, input, textarea, select, [contenteditable]:not([contenteditable=\"false\"]), .msg-actions";

function minorsRoomy() {
  const root = document.documentElement;
  return root.dataset.look === "studio" && root.dataset.fit !== "slim";
}

function minorsPointAnchor(x, y) {
  const rect = { left: x, right: x, top: y - 4, bottom: y - 4, width: 0, height: 0 };
  return { getBoundingClientRect: () => rect, closest: () => null };
}

function minorsBarDrops(messageTop, barTop, viewTop) {
  return messageTop + barTop < viewTop;
}

function minorsChatSizeLabel(size) {
  return `${Math.round(Math.max(12, Number(size) * 0.9375))}px`;
}

function minorsOwnMenu(target, msg) {
  for (let node = target; node && node !== msg; node = node.parentElement) {
    if (node.oncontextmenu) return true;
  }
  return false;
}

function minorsMessageMenu(e) {
  if (!minorsRoomy() || !e.target.closest) return;
  const msg = e.target.closest(".msg");
  if (!msg || e.target.closest(MINORS_EDITABLE) || minorsOwnMenu(e.target, msg)) return;
  if (window.getSelection && String(window.getSelection()).length > 0) return;
  const bar = msg.querySelector(":scope > .msg-actions");
  const message = bar && messageOfActions.get(bar);
  if (!message || !bar.querySelector(":scope > button[title=\"More\"]")) return;
  e.preventDefault();
  openMessageMenu(message, minorsPointAnchor(e.clientX, e.clientY));
}

function minorsScroller(node) {
  for (let up = node.parentElement; up; up = up.parentElement) {
    const flow = getComputedStyle(up).overflowY;
    if (flow === "auto" || flow === "scroll") return up;
  }
  return null;
}

function minorsPlaceBar(msg) {
  if (!msg || !msg.isConnected) return;
  const bar = msg.querySelector(":scope > .msg-actions");
  if (!bar || !minorsRoomy()) {
    delete msg.dataset.minorsLow;
    return;
  }
  delete msg.dataset.minorsLow;
  const view = minorsScroller(msg);
  const top = parseFloat(getComputedStyle(bar).top) || 0;
  const viewTop = view ? view.getBoundingClientRect().top : 0;
  if (minorsBarDrops(msg.getBoundingClientRect().top, top, viewTop)) msg.dataset.minorsLow = "";
}

function minorsHoverBar() {
  let hovered = null;
  document.addEventListener("mouseover", e => {
    const msg = e.target.closest ? e.target.closest(".msg") : null;
    if (msg === hovered) return;
    if (hovered) delete hovered.dataset.minorsLow;
    hovered = msg;
    minorsPlaceBar(msg);
  }, true);
  document.addEventListener("scroll", () => minorsPlaceBar(hovered), true);
}

function minorsFolder() {
  const box = document.getElementById("storage-path");
  if (!box) return () => {};
  let path = "";
  let title = "";
  const paint = () => {
    if (box.dataset.minorsLabel === undefined || box.textContent !== box.dataset.minorsLabel) {
      path = box.textContent;
      title = box.title;
    }
    if (minorsRoomy()) {
      if (!path) return;
      const label = folderLabel(path);
      if (box.textContent !== label) box.textContent = label;
      box.dataset.minorsLabel = label;
      if (box.hasAttribute("title")) box.removeAttribute("title");
    } else if (box.dataset.minorsLabel !== undefined) {
      delete box.dataset.minorsLabel;
      box.textContent = path;
      if (title) box.title = title;
    }
  };
  new MutationObserver(paint).observe(box, { childList: true, characterData: true, subtree: true, attributes: true, attributeFilter: ["title"] });
  paint();
  return paint;
}

function minorsChatSize() {
  const label = document.getElementById("chat-size-label");
  const range = document.getElementById("chat-size");
  if (!label || !range) return () => {};
  const paint = () => {
    if (minorsRoomy()) {
      const want = minorsChatSizeLabel(range.value);
      if (label.textContent !== want) label.textContent = want;
      label.dataset.minorsSize = "";
    } else if (label.dataset.minorsSize !== undefined) {
      delete label.dataset.minorsSize;
      label.textContent = `${range.value}px`;
    }
  };
  new MutationObserver(paint).observe(label, { childList: true, characterData: true, subtree: true });
  range.addEventListener("input", paint);
  paint();
  return paint;
}

function minorsPreviewTime() {
  const time = document.querySelector(".appearance-preview .msg-time");
  if (!time) return () => {};
  const was = time.textContent;
  const paint = () => {
    if (minorsRoomy()) {
      const noon = new Date();
      noon.setHours(12, 0, 0, 0);
      const want = formatTime(noon.toISOString());
      if (time.textContent !== want) time.textContent = want;
      time.dataset.minorsTime = "";
    } else if (time.dataset.minorsTime !== undefined) {
      delete time.dataset.minorsTime;
      time.textContent = was;
    }
  };
  paint();
  return paint;
}

let minorsStarted = false;

function studioMinors() {
  if (minorsStarted) return;
  minorsStarted = true;
  document.addEventListener("contextmenu", minorsMessageMenu, true);
  minorsHoverBar();
  const paints = [minorsFolder(), minorsChatSize(), minorsPreviewTime()];
  new MutationObserver(() => {
    for (const paint of paints) paint();
    if (!minorsRoomy()) document.querySelectorAll(".msg[data-minors-low]").forEach(msg => delete msg.dataset.minorsLow);
  }).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit", "data-look"] });
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { minorsPointAnchor, minorsBarDrops, minorsChatSizeLabel };
}
