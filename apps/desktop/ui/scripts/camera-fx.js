const CAMERA_BACKGROUNDS = ["off", "blur"];
const CAMERA_FX_DEFAULTS = { background: "off", mirror: true };
const MASK_EDGE_LOW = 0.3;
const MASK_EDGE_HIGH = 0.7;
const MASK_KEEP = 0.4;
const SEGMENT_BUDGET_MS = 20;
const SEGMENT_MAX_STRIDE = 4;
const COST_SMOOTHING = 0.2;

function parseCameraFx(raw) {
  const value = raw && typeof raw === "object" ? raw : {};
  return {
    background: CAMERA_BACKGROUNDS.includes(value.background) ? value.background : CAMERA_FX_DEFAULTS.background,
    mirror: typeof value.mirror === "boolean" ? value.mirror : CAMERA_FX_DEFAULTS.mirror,
  };
}

function cameraFxNeedsSegmenter(settings) {
  return parseCameraFx(settings).background !== "off";
}

function edgeStep(confidence) {
  const t = Math.min(1, Math.max(0, (confidence - MASK_EDGE_LOW) / (MASK_EDGE_HIGH - MASK_EDGE_LOW)));
  return t * t * (3 - 2 * t);
}

function writeMaskAlpha(confidences, previous, rgba, keep = MASK_KEEP) {
  const fresh = !previous.primed;
  for (let i = 0; i < confidences.length; i++) {
    const target = edgeStep(confidences[i]);
    const value = fresh ? target : previous.values[i] * keep + target * (1 - keep);
    previous.values[i] = value;
    const o = i * 4;
    rgba[o] = 255;
    rgba[o + 1] = 255;
    rgba[o + 2] = 255;
    rgba[o + 3] = Math.round(value * 255);
  }
  previous.primed = true;
}

function maskMemory(size) {
  return { values: new Float32Array(size), primed: false };
}

function segmentStride(costMs) {
  if (!(costMs > 0)) return 1;
  return Math.min(SEGMENT_MAX_STRIDE, Math.max(1, Math.ceil(costMs / SEGMENT_BUDGET_MS)));
}

function averageCost(previous, sample) {
  if (!(previous > 0)) return sample;
  return previous + (sample - previous) * COST_SMOOTHING;
}

function cameraFrameSize(width, height, maxWidth) {
  if (!width || !height) return null;
  const scale = Math.min(1, maxWidth / width);
  return { width: Math.round(width * scale), height: Math.round(height * scale) };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    CAMERA_BACKGROUNDS,
    parseCameraFx,
    cameraFxNeedsSegmenter,
    edgeStep,
    writeMaskAlpha,
    maskMemory,
    segmentStride,
    averageCost,
    cameraFrameSize,
  };
}
