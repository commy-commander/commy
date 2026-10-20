const CAMERA_FX_KEY = "commy.cameraEffects";
const CAMERA_BACKDROP_KEY = "commy.cameraBackdrop";
const CAMERA_BACKDROP_WIDTH = 960;
const CAMERA_BACKDROP_HEIGHT = 540;
const CAMERA_DRAW_FPS = 15;
const CAMERA_MAX_WIDTH = 640;
const CAMERA_SEGMENT_WIDTH = 256;
const CAMERA_SLOW_GPU_MS = 30;
const CAMERA_TIMING_RUNS = 4;
const CAMERA_SAMPLE_FILL = "#0b0d17";
const CAMERA_FX_FILES = {
  bundle: "vendor/mediapipe/vision_bundle.mjs",
  loader: "vendor/mediapipe/wasm/vision_wasm_internal.js",
  binary: "vendor/mediapipe/wasm/vision_wasm_internal.wasm",
  model: "vendor/mediapipe/selfie_segmenter.tflite",
};

const cameraFx = {
  settings: readCameraFx(),
  segmenter: null,
  loading: null,
  failed: false,
  lastTimestamp: 0,
  cost: 0,
  backdrop: null,
  listeners: new Set(),
};

loadCameraBackdrop(readCameraBackdrop());

function readCameraBackdrop() {
  try {
    return localStorage.getItem(CAMERA_BACKDROP_KEY);
  } catch {
    return null;
  }
}

function loadCameraBackdrop(src) {
  if (!src) return;
  const img = new Image();
  img.onload = () => {
    cameraFx.backdrop = img;
    notifyCameraFx();
  };
  img.src = src;
}

function hasCameraBackdrop() {
  return Boolean(cameraFx.backdrop || readCameraBackdrop());
}

function setCameraBackdrop(base64Jpeg) {
  const src = `data:image/jpeg;base64,${base64Jpeg}`;
  try { localStorage.setItem(CAMERA_BACKDROP_KEY, src); } catch {}
  loadCameraBackdrop(src);
}

function readCameraFx() {
  try {
    return parseCameraFx(JSON.parse(localStorage.getItem(CAMERA_FX_KEY) || "null"));
  } catch {
    return parseCameraFx(null);
  }
}

function updateCameraFx(patch) {
  cameraFx.settings = parseCameraFx({ ...cameraFx.settings, ...patch });
  try { localStorage.setItem(CAMERA_FX_KEY, JSON.stringify(cameraFx.settings)); } catch {}
  if (cameraFxNeedsSegmenter(cameraFx.settings)) prepareCameraSegmenter();
  notifyCameraFx();
}

function onCameraFxChange(listener) {
  cameraFx.listeners.add(listener);
}

function notifyCameraFx() {
  for (const listener of cameraFx.listeners) listener(cameraFxStatus());
}

function cameraFxStatus() {
  if (!cameraFxNeedsSegmenter(cameraFx.settings)) return "idle";
  if (cameraFx.failed) return "failed";
  if (cameraFx.segmenter) return "ready";
  return "loading";
}

function vendorUrl(path) {
  return new URL(path, document.baseURI).href;
}

async function createCameraSegmenter() {
  const { ImageSegmenter } = await import(vendorUrl(CAMERA_FX_FILES.bundle));
  const fileset = { wasmLoaderPath: vendorUrl(CAMERA_FX_FILES.loader), wasmBinaryPath: vendorUrl(CAMERA_FX_FILES.binary) };
  let lastError = null;
  for (const delegate of ["GPU", "CPU"]) {
    try {
      const segmenter = await ImageSegmenter.createFromOptions(fileset, {
        baseOptions: { modelAssetPath: vendorUrl(CAMERA_FX_FILES.model), delegate },
        runningMode: "VIDEO",
        outputCategoryMask: false,
        outputConfidenceMasks: true,
      });
      const cost = timeSegmenter(segmenter);
      if (delegate === "GPU" && cost > CAMERA_SLOW_GPU_MS) {
        segmenter.close();
        continue;
      }
      cameraFx.cost = cost;
      return segmenter;
    } catch (e) {
      lastError = e;
    }
  }
  throw lastError || new Error("no segmenter");
}

function timeSegmenter(segmenter) {
  const sample = canvasWithContext();
  sample.canvas.width = CAMERA_SEGMENT_WIDTH;
  sample.canvas.height = Math.round(CAMERA_SEGMENT_WIDTH * 9 / 16);
  sample.ctx.fillStyle = CAMERA_SAMPLE_FILL;
  sample.ctx.fillRect(0, 0, sample.canvas.width, sample.canvas.height);
  const run = () => {
    const result = segmenter.segmentForVideo(sample.canvas, nextSegmentTimestamp());
    for (const mask of result.confidenceMasks || []) {
      mask.getAsFloat32Array();
      mask.close();
    }
  };
  run();
  const started = performance.now();
  for (let i = 0; i < CAMERA_TIMING_RUNS; i++) run();
  return (performance.now() - started) / CAMERA_TIMING_RUNS;
}

function prepareCameraSegmenter() {
  if (cameraFx.segmenter || cameraFx.loading || cameraFx.failed) return;
  cameraFx.loading = createCameraSegmenter()
    .then(segmenter => { cameraFx.segmenter = segmenter; })
    .catch(reportCameraFxFailure)
    .finally(() => {
      cameraFx.loading = null;
      notifyCameraFx();
    });
}

function reportCameraFxFailure(error) {
  console.warn("Background effects unavailable", error);
  if (cameraFx.failed) return;
  cameraFx.failed = true;
  toast("Background effects are not available on this device, so your camera is blurred. Pick Off under Settings, then Voice & Camera.");
}

function nextSegmentTimestamp() {
  cameraFx.lastTimestamp = Math.max(performance.now(), cameraFx.lastTimestamp + 1);
  return cameraFx.lastTimestamp;
}

function canvasWithContext() {
  const canvas = document.createElement("canvas");
  return { canvas, ctx: canvas.getContext("2d") };
}

function createCameraPipeline(stream) {
  const video = document.createElement("video");
  video.muted = true;
  video.playsInline = true;
  video.srcObject = stream;
  video.play().catch(() => {});
  const out = canvasWithContext();
  out.canvas.className = "cam-self";
  const pipe = {
    stream,
    video,
    canvas: out.canvas,
    ctx: out.ctx,
    person: canvasWithContext(),
    mask: canvasWithContext(),
    small: canvasWithContext(),
    input: canvasWithContext(),
    frame: 0,
    drawn: false,
    maskImage: null,
    maskState: null,
    timer: null,
  };
  if (cameraFxNeedsSegmenter(cameraFx.settings)) prepareCameraSegmenter();
  pipe.timer = setInterval(() => drawCameraFrame(pipe), Math.round(1000 / CAMERA_DRAW_FPS));
  return pipe;
}

function stopCameraPipeline(pipe) {
  if (!pipe) return;
  clearInterval(pipe.timer);
  pipe.timer = null;
  for (const track of pipe.stream.getTracks()) track.stop();
  pipe.video.srcObject = null;
  pipe.canvas.remove();
}

function drawCameraFrame(pipe) {
  const size = cameraFrameSize(pipe.video.videoWidth, pipe.video.videoHeight, CAMERA_MAX_WIDTH);
  if (!size) return;
  if (pipe.canvas.width !== size.width || pipe.canvas.height !== size.height) {
    pipe.canvas.width = size.width;
    pipe.canvas.height = size.height;
  }
  pipe.drawn = true;
  const mode = cameraFx.settings.background;
  if (mode === "off") {
    pipe.ctx.drawImage(pipe.video, 0, 0, size.width, size.height);
    return;
  }
  if (!cameraFx.segmenter) {
    drawBlurredBackground(pipe, size);
    return;
  }
  try {
    drawPersonLayer(pipe, size);
  } catch (e) {
    cameraFx.segmenter = null;
    reportCameraFxFailure(e);
    notifyCameraFx();
    drawBlurredBackground(pipe, size);
    return;
  }
  if (mode === "image" && cameraFx.backdrop) drawBackdrop(pipe.ctx, size.width, size.height);
  else drawBlurredBackground(pipe, size);
  pipe.ctx.drawImage(pipe.person.canvas, 0, 0);
}

function drawPersonLayer(pipe, size) {
  pipe.frame += 1;
  if (!pipe.maskImage || pipe.frame % segmentStride(cameraFx.cost) === 0) segmentFrame(pipe, size);
  const layer = pipe.person;
  if (layer.canvas.width !== size.width || layer.canvas.height !== size.height) {
    layer.canvas.width = size.width;
    layer.canvas.height = size.height;
  }
  layer.ctx.globalCompositeOperation = "copy";
  layer.ctx.drawImage(pipe.video, 0, 0, size.width, size.height);
  layer.ctx.globalCompositeOperation = "destination-in";
  layer.ctx.imageSmoothingEnabled = true;
  layer.ctx.imageSmoothingQuality = "high";
  layer.ctx.drawImage(pipe.mask.canvas, 0, 0, size.width, size.height);
  layer.ctx.globalCompositeOperation = "source-over";
}

function segmentFrame(pipe, size) {
  const input = pipe.input;
  const height = Math.round(CAMERA_SEGMENT_WIDTH * size.height / size.width);
  if (input.canvas.width !== CAMERA_SEGMENT_WIDTH || input.canvas.height !== height) {
    input.canvas.width = CAMERA_SEGMENT_WIDTH;
    input.canvas.height = height;
  }
  input.ctx.drawImage(pipe.video, 0, 0, CAMERA_SEGMENT_WIDTH, height);
  const started = performance.now();
  const result = cameraFx.segmenter.segmentForVideo(input.canvas, nextSegmentTimestamp());
  const masks = result.confidenceMasks || [];
  try {
    const person = masks[masks.length - 1];
    if (!person) throw new Error("no person mask");
    paintMask(pipe, person.width, person.height, person.getAsFloat32Array());
  } finally {
    for (const mask of masks) mask.close();
  }
  cameraFx.cost = averageCost(cameraFx.cost, performance.now() - started);
}

function paintMask(pipe, width, height, confidences) {
  if (!pipe.maskImage || pipe.maskImage.width !== width || pipe.maskImage.height !== height) {
    pipe.mask.canvas.width = width;
    pipe.mask.canvas.height = height;
    pipe.maskImage = pipe.mask.ctx.createImageData(width, height);
    pipe.maskState = maskMemory(width * height);
  }
  writeMaskAlpha(confidences, pipe.maskState, pipe.maskImage.data);
  pipe.mask.ctx.putImageData(pipe.maskImage, 0, 0);
}

function canvasFilterWorks(ctx) {
  return typeof ctx.filter === "string";
}

function drawBlurredBackground(pipe, size) {
  const { ctx, video } = pipe;
  if (canvasFilterWorks(ctx)) {
    const radius = Math.max(6, Math.round(size.width / 40));
    ctx.filter = `blur(${radius}px)`;
    ctx.drawImage(video, -radius, -radius, size.width + radius * 2, size.height + radius * 2);
    ctx.filter = "none";
    return;
  }
  const small = pipe.small;
  small.canvas.width = Math.max(8, Math.round(size.width / 16));
  small.canvas.height = Math.max(5, Math.round(size.height / 16));
  small.ctx.imageSmoothingEnabled = true;
  small.ctx.drawImage(video, 0, 0, small.canvas.width, small.canvas.height);
  ctx.imageSmoothingEnabled = true;
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(small.canvas, 0, 0, size.width, size.height);
}

function drawBackdrop(ctx, width, height) {
  const img = cameraFx.backdrop;
  const crop = coverCrop(img.naturalWidth, img.naturalHeight, width, height);
  if (!crop) return;
  ctx.imageSmoothingEnabled = true;
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(img, crop.sx, crop.sy, crop.sw, crop.sh, 0, 0, width, height);
}

