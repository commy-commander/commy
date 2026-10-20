import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  AVATAR_REST,
  blendshapeScores,
  faceExpression,
  headAngles,
  faceRig,
  smoothRig,
  idleBlink,
  idleRig,
  vrmExpressionWeights,
} = require("../scripts/avatar-rig.js");

const close = (a, b, eps = 1e-6) => assert.ok(Math.abs(a - b) < eps, `${a} is not ${b}`);

function rotationY(angle, scale = 1) {
  const c = Math.cos(angle) * scale;
  const s = Math.sin(angle) * scale;
  return [c, 0, -s, 0, 0, scale, 0, 0, s, 0, c, 0, 0, 0, -40, 1];
}

test("face scores become a rig with each eye and the mouth on its own", () => {
  const scores = blendshapeScores([
    { categoryName: "eyeBlinkLeft", score: 0.9 },
    { categoryName: "jawOpen", score: 0.6 },
    { categoryName: "mouthSmileLeft", score: 0.4 },
    { categoryName: "mouthSmileRight", score: 0.8 },
    { categoryName: "eyeLookOutLeft", score: 0.6 },
    { categoryName: "eyeLookInRight", score: 0.6 },
  ]);
  const face = faceExpression(scores);
  close(face.blinkLeft, 0.9);
  close(face.blinkRight, 0);
  close(face.jaw, 0.6);
  close(face.smile, 0.6);
  close(face.lookX, 0.6);
  close(face.lookY, 0);
});

test("missing or odd scores stay inside zero to one", () => {
  const face = faceExpression({ jawOpen: 4, eyeBlinkLeft: -1, browInnerUp: "x" });
  assert.equal(face.jaw, 1);
  assert.equal(face.blinkLeft, 0);
  assert.equal(face.browUp, 0);
});

test("head turns are read from the face matrix and capped", () => {
  const turned = headAngles(rotationY(0.3, 2.5));
  close(turned.yaw, 0.3);
  close(turned.pitch, 0);
  close(turned.roll, 0);
  close(headAngles(rotationY(1.2)).yaw, 0.7);
  assert.deepEqual(headAngles(null), { pitch: 0, yaw: 0, roll: 0 });
});

test("a frame without a face gives no rig", () => {
  assert.equal(faceRig({ faceBlendshapes: [] }), null);
  const rig = faceRig({
    faceBlendshapes: [{ categories: [{ categoryName: "jawOpen", score: 0.5 }] }],
    facialTransformationMatrixes: [{ data: rotationY(0.2) }],
  });
  close(rig.jaw, 0.5);
  close(rig.yaw, 0.2);
});

test("smoothing eases toward the face and lets blinks through quickly", () => {
  const start = { ...AVATAR_REST };
  const next = smoothRig(start, { ...AVATAR_REST, jaw: 1, blinkLeft: 1 }, 0.45);
  close(next.jaw, 0.55);
  close(next.blinkLeft, 0.85);
  assert.deepEqual(smoothRig(null, { ...AVATAR_REST, jaw: 0.3 }), { ...AVATAR_REST, jaw: 0.3 });
});

test("the idle avatar blinks briefly every few seconds and sways gently", () => {
  assert.equal(idleBlink(1), 0);
  assert.ok(idleBlink(4.2 + 0.08) > 0.9);
  assert.equal(idleBlink(4.2 + 0.5), 0);
  for (let t = 0; t < 20; t += 0.25) {
    const rig = idleRig(t);
    assert.ok(Math.abs(rig.yaw) <= 0.08 && Math.abs(rig.pitch) <= 0.04);
    assert.equal(rig.jaw, 0);
  }
});

test("VRM expressions follow the mouth, smile, brows and eyes", () => {
  const rest = vrmExpressionWeights(AVATAR_REST);
  for (const value of Object.values(rest)) assert.equal(value, 0);
  const open = vrmExpressionWeights({ ...AVATAR_REST, jaw: 0.9, smile: 0.9, blinkRight: 1 });
  assert.equal(open.aa, 1);
  assert.ok(open.happy > 0.9);
  assert.equal(open.blinkRight, 1);
  assert.equal(open.blinkLeft, 0);
  assert.ok(vrmExpressionWeights({ ...AVATAR_REST, browUp: 1 }).surprised > 0.9);
});
