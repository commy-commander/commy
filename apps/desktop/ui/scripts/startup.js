const STARTUP_STEPS = ["connection", "update", "sync"];
const STARTUP_STATES = ["waiting", "working", "done", "skipped", "failed"];

function readStartupStep(step) {
  const s = step && typeof step === "object" ? step : {};
  const progress = Number.isFinite(s.progress) ? Math.max(0, Math.min(100, Math.round(s.progress))) : null;
  return {
    state: STARTUP_STATES.includes(s.state) ? s.state : "waiting",
    detail: typeof s.detail === "string" ? s.detail : "",
    progress,
  };
}

function readStartup(view) {
  if (!view || typeof view !== "object") return null;
  return {
    connection: readStartupStep(view.connection),
    update: readStartupStep(view.update),
    sync: readStartupStep(view.sync),
    offline: view.offline === true,
    restarting: view.restarting === true,
    ready: view.ready === true,
    canSkip: view.can_skip === true,
  };
}

function startupTitle(view) {
  if (!view) return "Starting Commy";
  if (view.restarting) return "Restarting to finish the update";
  if (view.ready) return view.offline ? "Opening Commy offline" : "Opening Commy";
  return "Starting Commy";
}

const STARTUP_MIN_SHOWN_MS = 2000;
const STARTUP_STEP_GAP_MS = 500;

function emptyStartupPace() {
  return { revealedAt: {}, working: {} };
}

function startupStepSettled(step) {
  return step.state !== "working" && step.state !== "waiting";
}

function earliestReveal(index, previousRevealedAt, shownAt) {
  const own = shownAt + (index + 1) * STARTUP_STEP_GAP_MS;
  if (index === 0) return own;
  if (previousRevealedAt == null) return Infinity;
  return Math.max(own, previousRevealedAt + STARTUP_STEP_GAP_MS);
}

function heldAsWorking(step, lastWorking) {
  if (!lastWorking) return { state: "working", detail: step.detail, progress: null };
  return { state: "working", detail: lastWorking.detail, progress: lastWorking.progress };
}

function pacedStartup(view, pace, shownAt, now) {
  if (view.restarting) return { view, pace, canClose: false };
  const revealedAt = { ...pace.revealedAt };
  const working = { ...pace.working };
  const paced = { ...view };
  STARTUP_STEPS.forEach((key, index) => {
    const step = view[key];
    const settled = startupStepSettled(step);
    if (!settled) working[key] = step;
    const due = now >= earliestReveal(index, revealedAt[STARTUP_STEPS[index - 1]], shownAt);
    if (revealedAt[key] == null && settled && due) revealedAt[key] = now;
    paced[key] = revealedAt[key] == null ? heldAsWorking(step, working[key]) : step;
  });
  const allShownSettled = STARTUP_STEPS.every(key => revealedAt[key] != null && startupStepSettled(view[key]));
  const canClose = view.ready && allShownSettled && now >= shownAt + STARTUP_MIN_SHOWN_MS;
  paced.ready = canClose;
  return { view: paced, pace: { revealedAt, working }, canClose };
}

function startupPreviewStep(state, detail, progress = null) {
  return { state, detail, progress };
}

function startupPreview(scene, ms) {
  const after = (at, before, then) => (ms < at ? before : then);
  if (scene === "offline") {
    const settled = ms >= 900;
    return {
      connection: after(900, startupPreviewStep("working", "Checking your connection"), startupPreviewStep("failed", "Offline. Opening what is saved on this device.")),
      update: after(900, startupPreviewStep("working", "Looking for a new version"), startupPreviewStep("skipped", "Checks again once you are online")),
      sync: after(900, startupPreviewStep("working", "Connecting to the sync service"), startupPreviewStep("skipped", "Your servers catch up once you are online")),
      offline: settled, restarting: false, ready: settled, can_skip: false,
    };
  }
  if (scene === "update") {
    const progress = Math.max(0, Math.min(100, Math.round((ms - 1000) / 30)));
    const restarting = ms >= 4000;
    const update = restarting
      ? startupPreviewStep("working", "Restarting into Commy 0.2.0")
      : after(1000, startupPreviewStep("working", "Looking for a new version"), startupPreviewStep("working", "Downloading Commy 0.2.0", progress));
    return {
      connection: after(500, startupPreviewStep("working", "Checking your connection"), startupPreviewStep("done", "You are online")),
      update,
      sync: after(2200, startupPreviewStep("working", "Catching up 2 servers"), startupPreviewStep("done", "Everything is up to date")),
      offline: false, restarting, ready: false, can_skip: !restarting && ms >= 2500,
    };
  }
  return {
    connection: after(500, startupPreviewStep("working", "Checking your connection"), startupPreviewStep("done", "You are online")),
    update: after(1300, startupPreviewStep("working", "Looking for a new version"), startupPreviewStep("done", "You have the latest version")),
    sync: after(2200, startupPreviewStep("working", "Catching up 2 servers"), startupPreviewStep("done", "Everything is up to date")),
    offline: false, restarting: false, ready: ms >= 2200, can_skip: false,
  };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    STARTUP_STEPS, STARTUP_STATES, STARTUP_MIN_SHOWN_MS, STARTUP_STEP_GAP_MS,
    readStartupStep, readStartup, startupTitle, startupPreview, emptyStartupPace, pacedStartup,
  };
}
