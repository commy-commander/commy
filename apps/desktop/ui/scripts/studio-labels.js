const LABELS_TIMING = { restMs: 380, warmMs: 600, watchMs: 250 };
const LABELS_SPACE = { gap: 10, edge: 8, arrowInset: 12 };
const LABELS_SIDES = ["top", "bottom", "right", "left"];
const LABELS_TARGET = "[title], [data-tip], button[aria-label], [role=\"button\"][aria-label], a[aria-label]";

const labelsHeld = new WeakMap();
let labelsNode = null;
let labelsAnchor = null;
let labelsTimer = 0;
let labelsWatch = 0;
let labelsCoolAt = 0;

function labelsText(fields) {
  const pick = value => (typeof value === "string" ? value.trim() : "");
  const tip = pick(fields.tip);
  if (tip) return tip;
  const title = pick(fields.title);
  if (title) return title;
  const aria = pick(fields.aria);
  if (aria && !pick(fields.text) && !fields.typing) return aria;
  return "";
}

function labelsFits(side, anchor, size, view) {
  const { gap, edge } = LABELS_SPACE;
  if (side === "top") return anchor.top - gap - size.height >= edge;
  if (side === "bottom") return anchor.bottom + gap + size.height <= view.height - edge;
  if (side === "right") return anchor.right + gap + size.width <= view.width - edge;
  return anchor.left - gap - size.width >= edge;
}

function labelsClamp(value, low, high) {
  return Math.max(low, Math.min(value, Math.max(low, high)));
}

function labelsPlace(anchor, size, view, wanted) {
  const { gap, edge } = LABELS_SPACE;
  const first = LABELS_SIDES.includes(wanted) ? wanted : "top";
  const order = [first, ...LABELS_SIDES.filter(side => side !== first)];
  const side = order.find(each => labelsFits(each, anchor, size, view)) || first;
  const centerX = anchor.left + anchor.width / 2 - size.width / 2;
  const centerY = anchor.top + anchor.height / 2 - size.height / 2;
  const spots = {
    top: [centerX, anchor.top - gap - size.height],
    bottom: [centerX, anchor.bottom + gap],
    right: [anchor.right + gap, centerY],
    left: [anchor.left - gap - size.width, centerY],
  };
  const [x, y] = spots[side];
  const left = Math.round(labelsClamp(x, edge, view.width - edge - size.width));
  const top = Math.round(labelsClamp(y, edge, view.height - edge - size.height));
  const across = side === "top" || side === "bottom";
  const aim = across ? anchor.left + anchor.width / 2 - left : anchor.top + anchor.height / 2 - top;
  const span = across ? size.width : size.height;
  const arrow = Math.round(labelsClamp(aim, LABELS_SPACE.arrowInset, span - LABELS_SPACE.arrowInset));
  return { side, left, top, arrow };
}

function labelsTyping(node) {
  return node.matches("input, textarea, select, [contenteditable=\"true\"]");
}

function labelsVisibleText(node) {
  return labelsTyping(node) ? "" : node.innerText || "";
}

function labelsOf(node) {
  const held = labelsHeld.get(node);
  return labelsText({
    tip: node.getAttribute("data-tip"),
    title: held ? held.title : node.getAttribute("title"),
    aria: node.getAttribute("aria-label"),
    text: labelsVisibleText(node),
    typing: labelsTyping(node),
  });
}

function labelsFind(start) {
  for (let node = start; node && node.nodeType === 1; node = node.parentElement) {
    if (node === labelsNode) return null;
    if (node.matches(LABELS_TARGET) && labelsOf(node)) return node;
  }
  return null;
}

function labelsHold(node) {
  if (labelsHeld.has(node) || !node.hasAttribute("title")) return;
  const title = node.getAttribute("title");
  const named = node.hasAttribute("aria-label");
  labelsHeld.set(node, { title, named });
  if (!named && !labelsVisibleText(node).trim()) node.setAttribute("aria-label", title);
  node.removeAttribute("title");
}

function labelsRelease(node) {
  const held = labelsHeld.get(node);
  if (!held) return;
  labelsHeld.delete(node);
  if (!held.named && node.getAttribute("aria-label") === held.title) node.removeAttribute("aria-label");
  if (!node.hasAttribute("title")) node.setAttribute("title", held.title);
}

function labelsBubble() {
  if (labelsNode && labelsNode.isConnected) return labelsNode;
  labelsNode = document.createElement("div");
  labelsNode.className = "studio-label";
  labelsNode.setAttribute("role", "tooltip");
  labelsNode.setAttribute("aria-hidden", "true");
  if (typeof labelsNode.showPopover === "function") labelsNode.setAttribute("popover", "manual");
  document.body.appendChild(labelsNode);
  return labelsNode;
}

function labelsRaise(bubble) {
  if (!bubble.hasAttribute("popover")) return;
  try {
    if (bubble.matches(":popover-open")) bubble.hidePopover();
    bubble.showPopover();
  } catch {
    bubble.removeAttribute("popover");
  }
}

function labelsSlim() {
  return document.documentElement.dataset.fit === "slim";
}

function labelsShow(node) {
  if (!node.isConnected || labelsSlim()) return;
  const text = labelsOf(node);
  if (!text) return;
  const bubble = labelsBubble();
  bubble.textContent = text;
  bubble.style.left = "0px";
  bubble.style.top = "0px";
  labelsRaise(bubble);
  bubble.classList.add("is-on");
  const rect = node.getBoundingClientRect();
  const size = { width: bubble.offsetWidth, height: bubble.offsetHeight };
  const view = { width: document.documentElement.clientWidth, height: document.documentElement.clientHeight };
  const spot = labelsPlace(rect, size, view, node.getAttribute("data-tip-side"));
  bubble.dataset.side = spot.side;
  bubble.style.left = `${spot.left}px`;
  bubble.style.top = `${spot.top}px`;
  bubble.style.setProperty("--label-arrow", `${spot.arrow}px`);
  clearInterval(labelsWatch);
  labelsWatch = setInterval(labelsCheck, LABELS_TIMING.watchMs);
}

function labelsCheck() {
  if (!labelsAnchor || !labelsAnchor.isConnected || !labelsAnchor.getClientRects().length) labelsHide();
}

function labelsHide() {
  clearTimeout(labelsTimer);
  clearInterval(labelsWatch);
  if (labelsNode && labelsNode.classList.contains("is-on")) {
    labelsCoolAt = Date.now();
    labelsNode.classList.remove("is-on");
    if (labelsNode.hasAttribute("popover")) {
      try { labelsNode.hidePopover(); } catch {}
    }
  }
  if (labelsAnchor) labelsRelease(labelsAnchor);
  labelsAnchor = null;
}

function labelsWarm() {
  return Date.now() - labelsCoolAt < LABELS_TIMING.warmMs || Boolean(labelsNode && labelsNode.classList.contains("is-on"));
}

function labelsArm(node, wait) {
  if (labelsAnchor === node) return;
  const warm = labelsWarm();
  labelsHide();
  labelsAnchor = node;
  labelsHold(node);
  if (warm || wait === 0) labelsShow(node);
  else labelsTimer = setTimeout(() => labelsAnchor === node && labelsShow(node), wait);
}

function labelsOver(e) {
  if (e.pointerType === "touch" || labelsSlim()) return;
  const node = labelsFind(e.target);
  if (node) labelsArm(node, LABELS_TIMING.restMs);
  else if (labelsAnchor && !labelsAnchor.contains(e.target)) labelsHide();
}

function labelsOut(e) {
  if (!labelsAnchor) return;
  const to = e.relatedTarget;
  if (!to || !labelsAnchor.contains(to)) labelsHide();
}

function labelsFocus(e) {
  const node = e.target;
  if (!node || !node.matches || !node.matches(":focus-visible")) return;
  const found = labelsFind(node);
  if (found === node && !labelsTyping(node)) labelsArm(node, 0);
}

function labelsBlur(e) {
  if (labelsAnchor && labelsAnchor === e.target) labelsHide();
}

function labelsKey(e) {
  if (e.key === "Escape" && labelsAnchor) labelsHide();
}

function studioLabels() {
  if (document.documentElement.hasAttribute("data-labels")) return;
  document.documentElement.setAttribute("data-labels", "");
  document.addEventListener("pointerover", labelsOver, true);
  document.addEventListener("pointerout", labelsOut, true);
  document.addEventListener("pointerdown", labelsHide, true);
  document.addEventListener("focusin", labelsFocus, true);
  document.addEventListener("focusout", labelsBlur, true);
  document.addEventListener("keydown", labelsKey, true);
  document.addEventListener("scroll", labelsHide, true);
  document.addEventListener("wheel", labelsHide, { capture: true, passive: true });
  window.addEventListener("blur", labelsHide);
  window.addEventListener("resize", labelsHide);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { LABELS_TIMING, LABELS_SPACE, LABELS_SIDES, labelsText, labelsPlace, labelsFits };
}
