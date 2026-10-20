const AVATAR_HEAD_LIMIT = 0.7;
const AVATAR_KEEP = 0.45;
const AVATAR_BLINK_EVERY = 4.2;
const AVATAR_BLINK_LENGTH = 0.16;

const AVATAR_REST = Object.freeze({
  blinkLeft: 0,
  blinkRight: 0,
  jaw: 0,
  smile: 0,
  pucker: 0,
  browUp: 0,
  browDown: 0,
  lookX: 0,
  lookY: 0,
  pitch: 0,
  yaw: 0,
  roll: 0,
});

function clamp01(value) {
  return Math.min(1, Math.max(0, Number(value) || 0));
}

function clampAngle(value, limit = AVATAR_HEAD_LIMIT) {
  return Math.min(limit, Math.max(-limit, Number(value) || 0));
}

function blendshapeScores(categories) {
  const scores = {};
  for (const category of categories || []) scores[category.categoryName] = category.score;
  return scores;
}

function faceExpression(scores) {
  const s = name => clamp01(scores[name]);
  const lookRight = (s("eyeLookOutRight") + s("eyeLookInLeft")) / 2;
  const lookLeft = (s("eyeLookOutLeft") + s("eyeLookInRight")) / 2;
  const lookUp = (s("eyeLookUpLeft") + s("eyeLookUpRight")) / 2;
  const lookDown = (s("eyeLookDownLeft") + s("eyeLookDownRight")) / 2;
  return {
    blinkLeft: s("eyeBlinkLeft"),
    blinkRight: s("eyeBlinkRight"),
    jaw: s("jawOpen"),
    smile: (s("mouthSmileLeft") + s("mouthSmileRight")) / 2,
    pucker: Math.max(s("mouthPucker"), s("mouthFunnel")),
    browUp: s("browInnerUp"),
    browDown: (s("browDownLeft") + s("browDownRight")) / 2,
    lookX: Math.max(-1, Math.min(1, lookLeft - lookRight)),
    lookY: Math.max(-1, Math.min(1, lookUp - lookDown)),
  };
}

function headAngles(matrix) {
  if (!matrix || matrix.length < 16) return { pitch: 0, yaw: 0, roll: 0 };
  const column = i => Math.hypot(matrix[i], matrix[i + 1], matrix[i + 2]) || 1;
  const sx = column(0);
  const sy = column(4);
  const sz = column(8);
  const m11 = matrix[0] / sx;
  const m12 = matrix[4] / sy;
  const m13 = matrix[8] / sz;
  const m22 = matrix[5] / sy;
  const m23 = matrix[9] / sz;
  const m32 = matrix[6] / sy;
  const m33 = matrix[10] / sz;
  const yaw = Math.asin(Math.min(1, Math.max(-1, m13)));
  const upright = Math.abs(m13) < 0.9999999;
  return {
    pitch: clampAngle(upright ? Math.atan2(-m23, m33) : Math.atan2(m32, m22)),
    yaw: clampAngle(yaw),
    roll: clampAngle(upright ? Math.atan2(-m12, m11) : 0),
  };
}

function faceRig(result) {
  const shapes = result && result.faceBlendshapes && result.faceBlendshapes[0];
  const matrix = result && result.facialTransformationMatrixes && result.facialTransformationMatrixes[0];
  if (!shapes) return null;
  return {
    ...faceExpression(blendshapeScores(shapes.categories)),
    ...headAngles(matrix && matrix.data),
  };
}

function smoothRig(previous, next, keep = AVATAR_KEEP) {
  const target = next || AVATAR_REST;
  if (!previous) return { ...target };
  const out = {};
  for (const key of Object.keys(AVATAR_REST)) {
    const blinkKey = key === "blinkLeft" || key === "blinkRight";
    const hold = blinkKey ? keep / 3 : keep;
    out[key] = previous[key] * hold + target[key] * (1 - hold);
  }
  return out;
}

function idleBlink(seconds) {
  const phase = ((seconds % AVATAR_BLINK_EVERY) + AVATAR_BLINK_EVERY) % AVATAR_BLINK_EVERY;
  if (phase > AVATAR_BLINK_LENGTH) return 0;
  return Math.sin((phase / AVATAR_BLINK_LENGTH) * Math.PI);
}

function idleRig(seconds) {
  const blink = idleBlink(seconds);
  return {
    ...AVATAR_REST,
    blinkLeft: blink,
    blinkRight: blink,
    yaw: Math.sin(seconds * 0.5) * 0.08,
    pitch: Math.sin(seconds * 0.37) * 0.04,
  };
}

function vrmExpressionWeights(rig) {
  const r = rig || AVATAR_REST;
  return {
    aa: clamp01(r.jaw * 1.2),
    ou: clamp01(r.pucker * (1 - r.jaw)),
    happy: clamp01((r.smile - 0.15) * 1.3),
    surprised: clamp01((r.browUp - 0.35) * 1.5),
    angry: clamp01((r.browDown - 0.4) * 1.5),
    blinkLeft: clamp01(r.blinkLeft),
    blinkRight: clamp01(r.blinkRight),
  };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    AVATAR_REST,
    blendshapeScores,
    faceExpression,
    headAngles,
    faceRig,
    smoothRig,
    idleBlink,
    idleRig,
    vrmExpressionWeights,
  };
}
