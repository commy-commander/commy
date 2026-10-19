const CAMERA_BACKGROUNDS = ["off", "blur", "stars"];
const CAMERA_FX_DEFAULTS = { background: "off", mirror: true };
const CAMERA_STAR_COUNT = 140;
const CAMERA_STAR_SEED = 20261014;
const CAMERA_TWINKLE_SECONDS = 6;
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

function seededRandom(seed) {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 4294967296;
  };
}

function cameraStars(count = CAMERA_STAR_COUNT, seed = CAMERA_STAR_SEED) {
  const next = seededRandom(seed);
  const stars = [];
  for (let i = 0; i < count; i++) {
    const size = next();
    stars.push({
      x: next(),
      y: next(),
      r: size < 0.82 ? 0.6 : size < 0.96 ? 1 : 1.5,
      alpha: 0.35 + next() * 0.6,
      twinkle: i % 4 === 0,
      phase: next(),
    });
  }
  return stars;
}

function starAlpha(star, seconds) {
  if (!star.twinkle) return star.alpha;
  const turn = seconds / CAMERA_TWINKLE_SECONDS + star.phase;
  const wave = (1 + Math.cos(turn * 2 * Math.PI)) / 2;
  return star.alpha * (0.45 + 0.55 * wave);
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
    cameraStars,
    starAlpha,
    edgeStep,
    writeMaskAlpha,
    maskMemory,
    segmentStride,
    averageCost,
    cameraFrameSize,
  };
}
