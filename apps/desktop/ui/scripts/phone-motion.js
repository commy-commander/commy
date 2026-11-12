const MOTION = {
  speedWindowMs: 100,
  flick: 0.45,
  flickTravel: 24,
  backShare: 0.33,
  sheetFlick: 0.5,
  sheetFlickTravel: 16,
  pressShowMs: 80,
  sheetOutMs: 200,
  paneMs: 340,
  paneMinMs: 160,
  paneInMs: 260,
  railFade: 4,
  railPad: 10,
  keyboardMin: 80,
  barShow: 28,
  barHide: 14,
  barEnd: 48,
  barTouchMs: 2500,
};
const MOVES = ["push", "pop"];
const PRESSABLE = ".msg, .channel, .dm-thread, .rail-item, .voice-user, .member, .srow, .arow";

function dragSpeed(samples, now) {
  const recent = samples.filter(sample => now - sample.t <= MOTION.speedWindowMs);
  if (recent.length < 2) return { x: 0, y: 0 };
  const first = recent[0];
  const last = recent[recent.length - 1];
  const ms = last.t - first.t;
  if (!(ms > 0)) return { x: 0, y: 0 };
  return { x: (last.x - first.x) / ms, y: (last.y - first.y) / ms };
}

function keepSamples(samples, sample) {
  const kept = samples.filter(old => sample.t - old.t <= MOTION.speedWindowMs * 2);
  kept.push(sample);
  return kept;
}

function swipeSettles(travel, speed, at, flickTravel = MOTION.flickTravel, flick = MOTION.flick) {
  if (speed <= -flick) return false;
  if (travel >= at) return true;
  return speed >= flick && travel >= flickTravel;
}

function backSettles(dx, speed, width) {
  return swipeSettles(dx, speed, Math.max(1, width) * MOTION.backShare);
}

function sheetSettles(dy, speed, at) {
  return swipeSettles(dy, speed, at, MOTION.sheetFlickTravel, MOTION.sheetFlick);
}

function settleMs(left, speed, longest = MOTION.paneMs, shortest = 140) {
  const pace = Math.abs(speed);
  if (!(pace > 0)) return longest;
  return Math.round(Math.max(shortest, Math.min(longest, (Math.abs(left) / pace) * 1.6)));
}

function viewportFrame({ fullHeight, layoutHeight, visibleHeight, offsetTop }) {
  const full = Math.max(0, Math.round(Number(fullHeight) || 0));
  const visible = Math.max(0, Math.round(Number(visibleHeight) || 0));
  const layout = Math.max(visible, Math.round(Number(layoutHeight) || 0) || full);
  const keyboard = full - visible >= MOTION.keyboardMin ? full - visible : 0;
  const lift = keyboard ? layout - visible : 0;
  const top = keyboard ? Math.max(0, Math.min(lift, Math.round(Number(offsetTop) || 0))) : 0;
  return { height: visible, top, keyboard, lift };
}

function fullHeightAfter(known, layoutHeight, visibleHeight, turned) {
  const seen = Math.max(Math.round(Number(layoutHeight) || 0), Math.round(Number(visibleHeight) || 0));
  return turned ? seen : Math.max(Number(known) || 0, seen);
}

function railFades(scrollLeft, clientWidth, scrollWidth) {
  const left = scrollLeft > MOTION.railFade;
  const right = scrollLeft + clientWidth < scrollWidth - MOTION.railFade;
  return { left, right };
}

function railReveal(itemLeft, itemWidth, scrollLeft, clientWidth, scrollWidth) {
  const pad = MOTION.railPad;
  const most = Math.max(0, scrollWidth - clientWidth);
  let next = scrollLeft;
  if (itemLeft - pad < scrollLeft) next = itemLeft - pad;
  else if (itemLeft + itemWidth + pad > scrollLeft + clientWidth) next = itemLeft + itemWidth + pad - clientWidth;
  return Math.max(0, Math.min(most, Math.round(next)));
}

function chatBarAfter(bar, delta, fromEnd) {
  if (fromEnd <= MOTION.barEnd) return { up: false, run: 0 };
  const run = Math.sign(delta) === Math.sign(bar.run) ? bar.run + delta : delta;
  if (run <= -MOTION.barShow) return { up: true, run: 0 };
  if (run >= MOTION.barHide) return { up: false, run: 0 };
  return { up: bar.up, run };
}

function moveKind(from, to) {
  if (from === to) return null;
  const depth = { list: 0, chat: 1, people: 2 };
  return (depth[to] ?? 0) > (depth[from] ?? 0) ? "push" : "pop";
}

let phoneMoveNext = null;
let phoneMoveCount = 0;

function primePhoneMove(next) {
  phoneMoveNext = next;
}

function runPhoneMove(kind, change) {
  const root = document.documentElement;
  const next = phoneMoveNext;
  phoneMoveNext = null;
  const apply = () => {
    change();
    if (next && next.land) next.land();
  };
  const calm = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const shown = MOVES.includes(kind) && !calm && root.dataset.fit === "slim" && root.dataset.look === "studio" && !document.hidden;
  if (!shown) {
    apply();
    return null;
  }
  const turn = ++phoneMoveCount;
  const settle = () => {
    if (turn !== phoneMoveCount) return;
    delete root.dataset.move;
    root.style.removeProperty("--pt-pop-from");
    root.style.removeProperty("--pt-move-ms");
  };
  if (typeof document.startViewTransition !== "function") {
    apply();
    root.dataset.move = `${kind}-in`;
    setTimeout(settle, MOTION.paneInMs);
    return null;
  }
  let done = false;
  const once = () => {
    if (done) return;
    done = true;
    apply();
  };
  root.dataset.move = kind;
  root.style.setProperty("--pt-pop-from", `${Math.max(0, Math.round((next && next.from) || 0))}px`);
  root.style.setProperty("--pt-move-ms", `${Math.round((next && next.ms) || MOTION.paneMs)}ms`);
  try {
    const move = document.startViewTransition(once);
    move.finished.then(settle, settle);
    move.ready.catch(() => {});
    return move;
  } catch (e) {
    once();
    settle();
    return null;
  }
}

function gripOf(view, fx, fy) {
  return { x: (fx - view.x) / view.scale, y: (fy - view.y) / view.scale };
}

function viewHolding(grip, scale, fx, fy) {
  return { scale, x: fx - grip.x * scale, y: fy - grip.y * scale };
}

function pinchScale(from, ratio, most) {
  return Math.min(most, Math.max(1, from * ratio));
}

function panWithin(at, half, centre, span) {
  if (half * 2 <= span) return 0;
  return Math.min(half - centre, Math.max(span - half - centre, at));
}

function pullFade(dx, dy, over) {
  return 1 - Math.min(0.6, Math.max(0, dx, dy) / over);
}

function touchOnly() {
  return typeof matchMedia === "function" && matchMedia("(hover: none) and (pointer: coarse)").matches;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { MOTION, MOVES, PRESSABLE, dragSpeed, keepSamples, swipeSettles, backSettles, sheetSettles, settleMs, viewportFrame, fullHeightAfter, railFades, railReveal, chatBarAfter, moveKind, gripOf, viewHolding, pinchScale, panWithin, pullFade, touchOnly };
}
