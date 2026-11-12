function phoneViewerFit() {
  const box = $("lightbox");
  const img = $("lightbox-img");
  if (studioSlim() && img.naturalWidth && img.naturalHeight) box.style.setProperty("--lb-ratio", img.naturalWidth / img.naturalHeight);
}

const phoneViewer = { dressed: false, step: false, shown: false };

function phoneViewerIcon(open, slim) {
  const icon = open.querySelector("svg");
  if (icon) icon.replaceWith(slim ? studioIconNode("arrow-up", 15) : svgIcon("openExternal", 16));
}

function phoneViewerDress() {
  const box = $("lightbox");
  const source = $("lightbox-source");
  const tray = box.querySelector(".lightbox-actions");
  const open = $("lightbox-open");
  const slim = studioSlim();
  if (!slim) {
    if (!phoneViewer.dressed) return;
    phoneViewer.dressed = false;
    box.insertBefore(source, tray);
    phoneViewerIcon(open, false);
    return;
  }
  if (!phoneViewer.dressed) {
    phoneViewer.dressed = true;
    tray.prepend(source);
    phoneViewerIcon(open, true);
  }
  const words = el("span", "", source.textContent);
  source.replaceChildren(...(source.classList.contains("linked") ? [svgIcon("link", 14)] : []), words);
  phoneViewerFit();
}

function viewerShown() {
  return studioSlim() && !$("lightbox").classList.contains("hidden");
}

function syncViewerStep() {
  const open = viewerShown();
  const step = layerStep("ptViewer", open, phoneViewer.step, history.state);
  if (step === "push") history.pushState({ ...(history.state || { pane: phonePane() }), ptViewer: true }, "");
  phoneViewer.step = open;
  if (step === "back") history.back();
}

const VIEWER = { most: 4, tap: 2.5, doubleMs: 280, doubleSlop: 30, slop: 8, awayAt: 110, fadeOver: 420, settleMs: 260, awayMs: 200 };
const VIEW_AT_REST = { scale: 1, x: 0, y: 0 };
const viewerTouch = { view: VIEW_AT_REST, drag: null, tapAt: 0, tapX: 0, tapY: 0, tapTimer: 0, awayTimer: 0 };

function viewerRest() {
  const view = viewerTouch.view;
  const box = $("lightbox-img").getBoundingClientRect();
  return { cx: box.left + box.width / 2 - view.x, cy: box.top + box.height / 2 - view.y, w: box.width / view.scale, h: box.height / view.scale };
}

function viewerHeld(view, rest) {
  return {
    scale: view.scale,
    x: panWithin(view.x, (rest.w * view.scale) / 2, rest.cx, window.innerWidth),
    y: panWithin(view.y, (rest.h * view.scale) / 2, rest.cy, window.innerHeight),
  };
}

function viewerPaint(view, ms = 0) {
  const img = $("lightbox-img");
  const rests = view.scale === 1 && !view.x && !view.y;
  img.style.transition = ms && !studioPhone.calm ? `transform ${ms}ms var(--ease-settle)` : "none";
  img.style.transform = rests ? "" : `translate(${view.x}px, ${view.y}px) scale(${view.scale})`;
  viewerTouch.view = rests ? VIEW_AT_REST : view;
}

function viewerReset() {
  const box = $("lightbox");
  const img = $("lightbox-img");
  clearTimeout(viewerTouch.tapTimer);
  clearTimeout(viewerTouch.awayTimer);
  viewerTouch.view = VIEW_AT_REST;
  viewerTouch.drag = null;
  viewerTouch.tapAt = 0;
  img.style.transition = "";
  img.style.transform = "";
  box.style.transition = "";
  box.style.opacity = "";
  box.classList.remove("pt-dragging");
}

function viewerGrab(touches, at, moved) {
  const rest = viewerRest();
  const view = viewerTouch.view;
  if (touches.length === 2) {
    const [a, b] = touches;
    const fx = (a.clientX + b.clientX) / 2 - rest.cx;
    const fy = (a.clientY + b.clientY) / 2 - rest.cy;
    return { kind: "pinch", rest, from: view, span: Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY) || 1, grip: gripOf(view, fx, fy), moved: true };
  }
  const touch = touches[0];
  return { kind: view.scale > 1 ? "pan" : "pull", rest, from: view, x: touch.clientX, y: touch.clientY, dx: 0, dy: 0, edge: edgeBackStarts(touch.clientX), moved, samples: [{ t: at, x: touch.clientX, y: touch.clientY }] };
}

function viewerTouchStart(e) {
  const was = viewerTouch.drag;
  viewerTouch.drag = null;
  if (!studioSlim() || e.touches.length > 2 || e.target.closest(".lightbox-actions")) return;
  clearTimeout(viewerTouch.tapTimer);
  const drag = viewerGrab(e.touches, e.timeStamp, e.touches.length > 1 || Boolean(was && was.moved));
  drag.onImage = e.touches.length === 1 && e.target === $("lightbox-img");
  if (drag.kind === "pinch" && was && was.kind === "pull" && was.moved) {
    $("lightbox").style.opacity = "";
    viewerPaint(VIEW_AT_REST);
    drag.from = VIEW_AT_REST;
    drag.rest = viewerRest();
  }
  viewerTouch.drag = drag;
}

function viewerTouchMove(e) {
  const drag = viewerTouch.drag;
  if (!drag) return;
  if (e.cancelable) e.preventDefault();
  const box = $("lightbox");
  if (drag.kind === "pinch") {
    if (e.touches.length < 2) return;
    const [a, b] = e.touches;
    const scale = pinchScale(drag.from.scale, Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY) / drag.span, VIEWER.most);
    box.classList.add("pt-dragging");
    viewerPaint(viewHolding(drag.grip, scale, (a.clientX + b.clientX) / 2 - drag.rest.cx, (a.clientY + b.clientY) / 2 - drag.rest.cy));
    return;
  }
  const touch = e.touches[0];
  drag.dx = touch.clientX - drag.x;
  drag.dy = touch.clientY - drag.y;
  if (!drag.moved && Math.hypot(drag.dx, drag.dy) < VIEWER.slop) return;
  drag.moved = true;
  drag.samples = keepSamples(drag.samples, { t: e.timeStamp, x: touch.clientX, y: touch.clientY });
  box.classList.add("pt-dragging");
  if (drag.kind === "pan") {
    viewerPaint(viewerHeld({ scale: drag.from.scale, x: drag.from.x + drag.dx, y: drag.from.y + drag.dy }, drag.rest));
    return;
  }
  viewerPaint({ scale: 1, x: drag.dx, y: drag.dy });
  box.style.transition = "none";
  box.style.opacity = String(pullFade(drag.edge ? drag.dx : 0, drag.dy, VIEWER.fadeOver));
}

function viewerLetGo(drag, at, cancelled) {
  const box = $("lightbox");
  const speed = dragSpeed(drag.samples, at);
  const down = !cancelled && sheetSettles(drag.dy, speed.y, VIEWER.awayAt);
  const back = !cancelled && !down && drag.edge && backSettles(drag.dx, speed.x, window.innerWidth);
  box.classList.remove("pt-dragging");
  if (!down && !back) {
    box.style.transition = "";
    box.style.opacity = "";
    viewerPaint(VIEW_AT_REST, VIEWER.settleMs);
    return;
  }
  const ms = studioPhone.calm ? 0 : VIEWER.awayMs;
  box.style.transition = `opacity ${ms}ms var(--ease-out)`;
  box.style.opacity = "0";
  viewerPaint(down ? { scale: 1, x: drag.dx, y: drag.dy + window.innerHeight / 2 } : { scale: 1, x: window.innerWidth, y: drag.dy }, ms);
  viewerTouch.awayTimer = setTimeout(closeLightbox, ms);
}

function viewerTap(e) {
  const touch = e.changedTouches[0];
  const twice = e.timeStamp - viewerTouch.tapAt < VIEWER.doubleMs && Math.hypot(touch.clientX - viewerTouch.tapX, touch.clientY - viewerTouch.tapY) < VIEWER.doubleSlop;
  if (!twice) {
    viewerTouch.tapAt = e.timeStamp;
    viewerTouch.tapX = touch.clientX;
    viewerTouch.tapY = touch.clientY;
    if (viewerTouch.view.scale === 1) viewerTouch.tapTimer = setTimeout(closeLightbox, VIEWER.doubleMs);
    return;
  }
  viewerTouch.tapAt = 0;
  const view = viewerTouch.view;
  if (view.scale > 1) {
    viewerPaint(VIEW_AT_REST, VIEWER.settleMs);
    return;
  }
  const rest = viewerRest();
  const fx = touch.clientX - rest.cx;
  const fy = touch.clientY - rest.cy;
  viewerPaint(viewerHeld(viewHolding(gripOf(view, fx, fy), VIEWER.tap, fx, fy), rest), VIEWER.settleMs);
}

function viewerTouchEnd(e, cancelled) {
  const drag = viewerTouch.drag;
  if (!drag) return;
  if (e.touches.length) {
    if (e.cancelable) e.preventDefault();
    viewerTouch.drag = viewerGrab([e.touches[0]], e.timeStamp, true);
    return;
  }
  viewerTouch.drag = null;
  if (drag.moved) {
    if (e.cancelable) e.preventDefault();
    viewerTouch.tapAt = 0;
    if (drag.kind === "pull") return viewerLetGo(drag, e.timeStamp, cancelled);
    $("lightbox").classList.remove("pt-dragging");
    const view = viewerTouch.view;
    viewerPaint(view.scale < 1.04 ? VIEW_AT_REST : viewerHeld(view, drag.rest), VIEWER.settleMs);
    return;
  }
  if (cancelled || !drag.onImage) return;
  if (e.cancelable) e.preventDefault();
  viewerTap(e);
}

function wirePhoneViewer() {
  const box = $("lightbox");
  if (!box) return;
  $("lightbox-img").addEventListener("load", phoneViewerFit);
  new MutationObserver(() => {
    const shown = !box.classList.contains("hidden");
    if (shown === phoneViewer.shown) return;
    phoneViewer.shown = shown;
    viewerReset();
    if (shown) phoneViewerDress();
    syncViewerStep();
  }).observe(box, { attributes: true, attributeFilter: ["class"] });
  box.addEventListener("touchstart", viewerTouchStart, { passive: true });
  box.addEventListener("touchmove", viewerTouchMove, { passive: false });
  box.addEventListener("touchend", e => viewerTouchEnd(e, false), { passive: false });
  box.addEventListener("touchcancel", e => viewerTouchEnd(e, true), { passive: false });
  window.addEventListener("popstate", () => {
    if ((history.state && history.state.ptViewer) || !viewerShown()) return;
    phoneViewer.step = false;
    closeLightbox();
  });
}

wirePhoneViewer();
