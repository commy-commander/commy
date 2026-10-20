const AVATAR_FILES = {
  scene: "scripts/avatar-scene.mjs",
  model: "vendor/mediapipe/face_landmarker.task",
};
const AVATAR_TRACK_WIDTH = 320;
const AVATAR_LOST_AFTER_MS = 1200;
const AVATAR_DB = "commy-camera";
const AVATAR_STORE = "avatars";
const AVATAR_CUSTOM_KEY = "custom";
const AVATAR_SKY_TOP = "#1f2438";
const AVATAR_SKY_BOTTOM = "#0b0d17";

const cameraAvatar = {
  stage: null,
  stageLoading: null,
  stageFailed: false,
  tracker: null,
  trackerLoading: null,
  trackerFailed: false,
  customLoading: null,
  lastTimestamp: 0,
  lastSeen: 0,
  cost: 0,
  rig: null,
  thumbnails: {},
};

function avatarDb() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(AVATAR_DB, 1);
    request.onupgradeneeded = () => request.result.createObjectStore(AVATAR_STORE);
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function avatarStoreRequest(mode, run) {
  const db = await avatarDb();
  try {
    return await new Promise((resolve, reject) => {
      const request = run(db.transaction(AVATAR_STORE, mode).objectStore(AVATAR_STORE));
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
  } finally {
    db.close();
  }
}

function readCustomAvatar() {
  return avatarStoreRequest("readonly", store => store.get(AVATAR_CUSTOM_KEY)).catch(() => null);
}

function writeCustomAvatar(bytes) {
  return avatarStoreRequest("readwrite", store => store.put(bytes, AVATAR_CUSTOM_KEY));
}

function prepareAvatarStage() {
  if (cameraAvatar.stage || cameraAvatar.stageLoading || cameraAvatar.stageFailed) return cameraAvatar.stageLoading;
  cameraAvatar.stageLoading = import(vendorUrl(AVATAR_FILES.scene))
    .then(({ createAvatarStage }) => createAvatarStage({ expressionWeights: vrmExpressionWeights }))
    .then(async stage => {
      cameraAvatar.stage = stage;
      await restoreCustomAvatar();
      renderAvatarThumbnails();
    })
    .catch(reportAvatarFailure)
    .finally(() => {
      cameraAvatar.stageLoading = null;
      notifyCameraFx();
    });
  return cameraAvatar.stageLoading;
}

async function restoreCustomAvatar() {
  const bytes = await readCustomAvatar();
  if (!bytes) return;
  try {
    await cameraAvatar.stage.setCustom(bytes);
  } catch (e) {
    console.warn("Saved avatar could not be loaded", e);
  }
}

function reportAvatarFailure(error) {
  console.warn("Camera avatars unavailable", error);
  if (cameraAvatar.stageFailed) return;
  cameraAvatar.stageFailed = true;
  if (cameraFxShowsAvatar(cameraFx.settings)) toast("Avatars are not available on this device, so your camera is blurred. Pick Off under Avatar in Settings, then Voice & Camera.");
}

async function createFaceTracker() {
  const { FaceLandmarker } = await import(vendorUrl(CAMERA_FX_FILES.bundle));
  const fileset = { wasmLoaderPath: vendorUrl(CAMERA_FX_FILES.loader), wasmBinaryPath: vendorUrl(CAMERA_FX_FILES.binary) };
  let lastError = null;
  for (const delegate of ["GPU", "CPU"]) {
    try {
      return await FaceLandmarker.createFromOptions(fileset, {
        baseOptions: { modelAssetPath: vendorUrl(AVATAR_FILES.model), delegate },
        runningMode: "VIDEO",
        numFaces: 1,
        outputFaceBlendshapes: true,
        outputFacialTransformationMatrixes: true,
      });
    } catch (e) {
      lastError = e;
    }
  }
  throw lastError || new Error("no face tracker");
}

function prepareFaceTracker() {
  if (cameraAvatar.tracker || cameraAvatar.trackerLoading || cameraAvatar.trackerFailed) return;
  cameraAvatar.trackerLoading = createFaceTracker()
    .then(tracker => { cameraAvatar.tracker = tracker; })
    .catch(e => {
      console.warn("Face tracking unavailable", e);
      cameraAvatar.trackerFailed = true;
    })
    .finally(() => {
      cameraAvatar.trackerLoading = null;
      notifyCameraFx();
    });
}

function prepareCameraAvatar() {
  prepareAvatarStage();
  prepareFaceTracker();
}

function cameraAvatarStatus() {
  if (cameraAvatar.stageFailed) return "failed";
  if (!cameraAvatar.stage) return "loading";
  if (cameraFx.settings.avatar === "custom" && !cameraAvatar.stage.hasCustom()) return "missing";
  if (cameraAvatar.trackerFailed) return "still";
  if (!cameraAvatar.tracker) return "loading";
  return "ready";
}

function nextTrackTimestamp() {
  cameraAvatar.lastTimestamp = Math.max(performance.now(), cameraAvatar.lastTimestamp + 1);
  return cameraAvatar.lastTimestamp;
}

function trackFace(pipe, size) {
  const input = pipe.input;
  const height = Math.round(AVATAR_TRACK_WIDTH * size.height / size.width);
  if (input.canvas.width !== AVATAR_TRACK_WIDTH || input.canvas.height !== height) {
    input.canvas.width = AVATAR_TRACK_WIDTH;
    input.canvas.height = height;
  }
  input.ctx.drawImage(pipe.video, 0, 0, AVATAR_TRACK_WIDTH, height);
  const started = performance.now();
  const rig = faceRig(cameraAvatar.tracker.detectForVideo(input.canvas, nextTrackTimestamp()));
  cameraAvatar.cost = averageCost(cameraAvatar.cost, performance.now() - started);
  if (rig) cameraAvatar.lastSeen = performance.now();
  return rig;
}

function currentAvatarRig(pipe, size, seconds) {
  let seen = null;
  if (cameraAvatar.tracker) {
    pipe.frame += 1;
    if (pipe.frame % segmentStride(cameraAvatar.cost) === 0 || !cameraAvatar.rig) {
      try {
        seen = trackFace(pipe, size);
      } catch (e) {
        console.warn("Face tracking stopped", e);
        cameraAvatar.tracker = null;
        cameraAvatar.trackerFailed = true;
        notifyCameraFx();
      }
    }
  }
  const lost = performance.now() - cameraAvatar.lastSeen > AVATAR_LOST_AFTER_MS;
  const target = seen || (lost ? idleRig(seconds) : null);
  if (target) cameraAvatar.rig = smoothRig(cameraAvatar.rig, target);
  return cameraAvatar.rig || idleRig(seconds);
}

function drawAvatarSky(ctx, width, height) {
  const sky = ctx.createLinearGradient(0, 0, 0, height);
  sky.addColorStop(0, AVATAR_SKY_TOP);
  sky.addColorStop(1, AVATAR_SKY_BOTTOM);
  ctx.fillStyle = sky;
  ctx.fillRect(0, 0, width, height);
}

function drawAvatarFrame(pipe, size) {
  prepareCameraAvatar();
  if (cameraAvatar.stageFailed) {
    drawBlurredBackground(pipe, size);
    return;
  }
  if (cameraFx.settings.background === "image" && cameraFx.backdrop) drawBackdrop(pipe.ctx, size.width, size.height);
  else drawAvatarSky(pipe.ctx, size.width, size.height);
  const stage = cameraAvatar.stage;
  if (!stage) return;
  const seconds = performance.now() / 1000;
  try {
    const drawn = stage.draw(cameraFx.settings.avatar, currentAvatarRig(pipe, size, seconds), seconds, size.width, size.height);
    if (drawn) pipe.ctx.drawImage(drawn, 0, 0, size.width, size.height);
  } catch (e) {
    cameraAvatar.stage = null;
    reportAvatarFailure(e);
    notifyCameraFx();
  }
}

function renderAvatarThumbnails() {
  const stage = cameraAvatar.stage;
  if (!stage) return;
  const rest = idleRig(1);
  for (const kind of CAMERA_AVATARS) {
    if (kind === "off" || (kind === "custom" && !stage.hasCustom())) continue;
    try {
      cameraAvatar.thumbnails[kind] = stage.snapshot(kind, rest);
    } catch {}
  }
  notifyCameraFx();
}

async function setCustomAvatar(bytes) {
  await prepareAvatarStage();
  if (!cameraAvatar.stage) throw new Error("Avatars are not available on this device");
  await cameraAvatar.stage.setCustom(bytes);
  await writeCustomAvatar(bytes);
  delete cameraAvatar.thumbnails.custom;
  renderAvatarThumbnails();
}
