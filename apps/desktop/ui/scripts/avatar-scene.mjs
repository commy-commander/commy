import {
  WebGLRenderer,
  Scene,
  PerspectiveCamera,
  Group,
  Mesh,
  SphereGeometry,
  CylinderGeometry,
  ConeGeometry,
  CapsuleGeometry,
  CircleGeometry,
  MeshToonMaterial,
  MeshBasicMaterial,
  DataTexture,
  RedFormat,
  NearestFilter,
  HemisphereLight,
  DirectionalLight,
  Color,
  Vector3,
  Euler,
  BackSide,
  SRGBColorSpace,
  LoadingManager,
  RoundedBoxGeometry,
  GLTFLoader,
  VRMLoaderPlugin,
  VRMUtils,
} from "../vendor/three/three_bundle.mjs";

const INK = "#0b0d17";
const CREAM = "#f2efe6";
const RED = "#ff4d4d";
const OUTLINE_SCALE = 1.045;
const VRM_FRAME_DISTANCE = 1.15;
const VRM_FRAME_DROP = 0.04;
const VRM_ARM_DROP = 1.25;

function toonRamp() {
  const ramp = new DataTexture(new Uint8Array([150, 205, 255]), 3, 1, RedFormat);
  ramp.minFilter = NearestFilter;
  ramp.magFilter = NearestFilter;
  ramp.needsUpdate = true;
  return ramp;
}

function makePalette() {
  const ramp = toonRamp();
  const cache = new Map();
  const toon = color => {
    if (!cache.has(color)) cache.set(color, new MeshToonMaterial({ color: new Color(color), gradientMap: ramp }));
    return cache.get(color);
  };
  const flat = (color, opacity = 1) => new MeshBasicMaterial({ color: new Color(color), transparent: opacity < 1, opacity });
  const outline = new MeshBasicMaterial({ color: new Color(INK), side: BackSide });
  return { toon, flat, outline };
}

function part(geometry, material, position = [0, 0, 0], scale = [1, 1, 1]) {
  const mesh = new Mesh(geometry, material);
  mesh.position.set(...position);
  mesh.scale.set(...scale);
  return mesh;
}

function outlined(palette, geometry, color, position, scale = [1, 1, 1]) {
  const group = new Group();
  group.position.set(...position);
  group.add(part(geometry, palette.toon(color), [0, 0, 0], scale));
  group.add(part(geometry, palette.outline, [0, 0, 0], scale.map(v => v * OUTLINE_SCALE)));
  return group;
}

function surfaceZ(x, y, radius = 1, depth = 1) {
  return Math.sqrt(Math.max(0, radius * radius - x * x - y * y)) * depth;
}

function makeEye(palette, { x, y, z, width, height, iris = INK, white = true }) {
  const eye = new Group();
  eye.position.set(x, y, z);
  const sphere = new SphereGeometry(1, 24, 16);
  if (white) eye.add(part(sphere, palette.flat(CREAM), [0, 0, 0], [width, height, width * 0.35]));
  const pupil = new Group();
  const pupilScale = white ? 0.62 : 1;
  pupil.add(part(sphere, palette.flat(iris), [0, 0, width * 0.12], [width * pupilScale, height * pupilScale, width * 0.3]));
  pupil.add(part(sphere, palette.flat("#ffffff"), [width * 0.2, height * 0.26, width * 0.4], [width * 0.26, width * 0.26, width * 0.12]));
  eye.add(pupil);
  return { eye, pupil, reach: width * (white ? 0.3 : 0.12) };
}

function makeMouth(palette, { y, z, width, color = "#3a1420" }) {
  const mouth = new Group();
  mouth.position.set(0, y, z);
  mouth.add(part(new CircleGeometry(1, 32), palette.flat(color), [0, 0, 0], [width, width * 0.5, 1]));
  mouth.add(part(new CircleGeometry(1, 32), palette.flat(RED), [0, -width * 0.18, 0.002], [width * 0.55, width * 0.22, 1]));
  return mouth;
}

function makeBrow(palette, { x, y, z, width, color = INK }) {
  const brow = part(new CapsuleGeometry(0.026, width, 4, 12), palette.flat(color), [x, y, z]);
  brow.rotation.z = Math.PI / 2;
  return brow;
}

function makeBody(palette, color) {
  const body = new Group();
  body.add(outlined(palette, new SphereGeometry(1, 40, 24), color, [0, -0.85, 0], [1.25, 0.95, 0.85]));
  return body;
}

function finishAvatar(parts) {
  const root = new Group();
  root.add(parts.body);
  parts.neck.add(parts.head);
  root.add(parts.neck);
  return { kind: "builtin", root, ...parts, frame: { y: 1.05, distance: 6.6 } };
}

function makeFaceParts(palette, head, options) {
  const { eyeY, eyeX, eyeWidth, eyeHeight, mouthY, mouthWidth, browY, depth = 1, iris, white } = options;
  const eyes = [-1, 1].map(side =>
    makeEye(palette, { x: side * eyeX, y: eyeY, z: surfaceZ(eyeX, eyeY, 1, depth) + 0.01, width: eyeWidth, height: eyeHeight, iris, white }));
  for (const { eye } of eyes) head.add(eye);
  const mouth = makeMouth(palette, { y: mouthY, z: surfaceZ(0, mouthY, 1, depth) + 0.015, width: mouthWidth });
  head.add(mouth);
  const brows = [-1, 1].map(side =>
    makeBrow(palette, { x: side * eyeX, y: browY, z: surfaceZ(eyeX, browY, 1, depth) + 0.02, width: eyeWidth * 1.1, color: options.browColor }));
  for (const brow of brows) head.add(brow);
  return { eyes, mouth, brows, browBase: browY };
}

function addCheeks(palette, head, { x, y, depth = 1 }) {
  for (const side of [-1, 1]) {
    const cheek = part(new CircleGeometry(1, 24), palette.flat(RED, 0.35), [side * x, y, surfaceZ(x, y, 1, depth) + 0.012], [0.16, 0.09, 1]);
    cheek.lookAt(cheek.position.clone().multiplyScalar(2));
    head.add(cheek);
  }
}

function buildCat(palette) {
  const neck = new Group();
  neck.position.set(0, -0.15, 0);
  const head = new Group();
  head.position.set(0, 1.15, 0);
  head.add(outlined(palette, new SphereGeometry(1, 48, 32), CREAM, [0, 0, 0], [1.08, 0.94, 1]));
  for (const side of [-1, 1]) {
    const ear = outlined(palette, new ConeGeometry(0.34, 0.62, 32), CREAM, [side * 0.6, 0.82, -0.05]);
    ear.rotation.z = -side * 0.42;
    ear.add(part(new ConeGeometry(0.2, 0.4, 32), palette.toon("#ff8f8f"), [0, -0.06, 0.12]));
    head.add(ear);
  }
  const face = makeFaceParts(palette, head, {
    eyeX: 0.38, eyeY: 0.06, eyeWidth: 0.16, eyeHeight: 0.21, mouthY: -0.36, mouthWidth: 0.12, browY: 0.36, iris: "#1b2140", white: false,
  });
  head.add(part(new SphereGeometry(1, 20, 12), palette.toon(RED), [0, -0.17, surfaceZ(0, -0.17) + 0.02], [0.08, 0.055, 0.05]));
  for (const side of [-1, 1]) {
    for (const tilt of [-0.12, 0.08]) {
      const whisker = part(new CylinderGeometry(0.008, 0.008, 0.42, 6), palette.flat(INK), [side * 0.62, -0.22 + tilt * 0.6, surfaceZ(0.55, -0.2) + 0.03]);
      whisker.rotation.z = Math.PI / 2 + side * tilt;
      head.add(whisker);
    }
  }
  addCheeks(palette, head, { x: 0.58, y: -0.2 });
  return finishAvatar({ neck, head, body: makeBody(palette, "#2b3160"), ...face });
}

function buildBot(palette) {
  const neck = new Group();
  neck.position.set(0, -0.15, 0);
  const head = new Group();
  head.position.set(0, 1.12, 0);
  const shell = new RoundedBoxGeometry(2, 1.7, 1.7, 6, 0.42);
  head.add(outlined(palette, shell, "#4a4f8f", [0, 0, 0]));
  head.add(part(new RoundedBoxGeometry(1.6, 1.12, 0.2, 5, 0.1), palette.toon(INK), [0, -0.02, 0.82]));
  const glow = "#f2efe6";
  const eyes = [-1, 1].map(side => {
    const eye = new Group();
    eye.position.set(side * 0.36, 0.08, 0.94);
    const pupil = new Group();
    pupil.add(part(new CapsuleGeometry(0.1, 0.12, 6, 16), palette.flat(glow)));
    eye.add(pupil);
    head.add(eye);
    return { eye, pupil, reach: 0.06 };
  });
  const mouth = new Group();
  mouth.position.set(0, -0.3, 0.94);
  mouth.add(part(new RoundedBoxGeometry(0.42, 0.12, 0.02, 3, 0.05), palette.flat(glow)));
  head.add(mouth);
  const brows = [-1, 1].map(side => {
    const brow = part(new RoundedBoxGeometry(0.26, 0.05, 0.02, 2, 0.02), palette.flat(RED), [side * 0.36, 0.34, 0.94]);
    head.add(brow);
    return brow;
  });
  head.add(part(new CylinderGeometry(0.04, 0.04, 0.4, 12), palette.toon("#9aa0d8"), [0, 1.02, 0]));
  head.add(outlined(palette, new SphereGeometry(0.12, 24, 16), RED, [0, 1.26, 0]));
  for (const side of [-1, 1]) {
    const bolt = outlined(palette, new CylinderGeometry(0.16, 0.16, 0.14, 24), "#9aa0d8", [side * 1.04, 0, 0]);
    bolt.rotation.z = Math.PI / 2;
    head.add(bolt);
  }
  return finishAvatar({ neck, head, body: makeBody(palette, "#2b3160"), eyes, mouth, brows, browBase: 0.34, boxy: true });
}

function buildAlien(palette) {
  const neck = new Group();
  neck.position.set(0, -0.15, 0);
  const head = new Group();
  head.position.set(0, 1.15, 0);
  const skin = "#58b8c9";
  head.add(outlined(palette, new SphereGeometry(1, 48, 32), skin, [0, 0, 0], [1, 1.08, 0.96]));
  for (const side of [-1, 1]) {
    const stalk = part(new CylinderGeometry(0.035, 0.05, 0.55, 12), palette.toon(skin), [side * 0.36, 1.12, 0]);
    stalk.rotation.z = -side * 0.35;
    head.add(stalk);
    head.add(outlined(palette, new SphereGeometry(0.11, 24, 16), CREAM, [side * 0.47, 1.38, 0]));
  }
  const face = makeFaceParts(palette, head, {
    eyeX: 0.36, eyeY: 0.1, eyeWidth: 0.22, eyeHeight: 0.27, mouthY: -0.38, mouthWidth: 0.1, browY: 0.46, depth: 0.96, iris: "#141a36", browColor: "#235d6b",
  });
  addCheeks(palette, head, { x: 0.6, y: -0.18, depth: 0.96 });
  return finishAvatar({ neck, head, body: makeBody(palette, "#3b2f6b"), ...face });
}

const BUILDERS = { cat: buildCat, bot: buildBot, alien: buildAlien };

function poseBuiltin(avatar, rig, seconds) {
  const { neck, head, eyes, mouth, brows, browBase } = avatar;
  neck.rotation.set(rig.pitch * 0.35, rig.yaw * 0.35, rig.roll * 0.35);
  head.rotation.set(rig.pitch * 0.65, rig.yaw * 0.65, rig.roll * 0.65);
  avatar.root.position.y = Math.sin(seconds * 1.6) * 0.015;
  const blinks = [rig.blinkRight, rig.blinkLeft];
  eyes.forEach(({ eye, pupil, reach }, i) => {
    const open = Math.max(0.08, 1 - blinks[i] * 1.1);
    eye.scale.set(1, open, 1);
    pupil.position.set(rig.lookX * reach, rig.lookY * reach, 0);
  });
  if (avatar.boxy) {
    mouth.scale.set(1 + rig.smile * 0.4 - rig.pucker * 0.45, 1 + rig.jaw * 2.6, 1);
  } else {
    mouth.scale.set(1.2 + rig.smile * 0.5 - rig.pucker * 0.45, 0.45 + rig.jaw * 1.5 + rig.pucker * 0.3, 1);
  }
  const lift = rig.browUp * 0.08 - rig.browDown * 0.06;
  brows.forEach((brow, i) => {
    brow.position.y = browBase + lift;
    const tilt = rig.browDown * 0.3 - rig.browUp * 0.12;
    brow.rotation.z = Math.PI / 2 * (avatar.boxy ? 0 : 1) + (i === 0 ? -tilt : tilt);
  });
}

function rejectRemoteUrls(url) {
  return url.startsWith("data:") || url.startsWith("blob:") ? url : "data:,";
}

async function loadVrm(bytes) {
  const manager = new LoadingManager();
  manager.setURLModifier(rejectRemoteUrls);
  const loader = new GLTFLoader(manager);
  loader.register(parser => new VRMLoaderPlugin(parser));
  const gltf = await loader.parseAsync(bytes, "");
  const vrm = gltf.userData.vrm;
  if (!vrm || !vrm.humanoid) throw new Error("That file is not a VRM avatar");
  VRMUtils.removeUnnecessaryVertices(gltf.scene);
  if (VRMUtils.combineSkeletons) VRMUtils.combineSkeletons(gltf.scene);
  VRMUtils.rotateVRM0(vrm);
  for (const object of [gltf.scene]) object.traverse(node => { node.frustumCulled = false; });
  const bone = name => vrm.humanoid.getNormalizedBoneNode(name);
  const leftArm = bone("leftUpperArm");
  const rightArm = bone("rightUpperArm");
  if (leftArm) leftArm.rotation.z = -VRM_ARM_DROP;
  if (rightArm) rightArm.rotation.z = VRM_ARM_DROP;
  vrm.update(0);
  const headNode = bone("head");
  const headPosition = new Vector3();
  if (headNode) headNode.getWorldPosition(headPosition);
  else headPosition.set(0, 1.4, 0);
  return {
    kind: "vrm",
    vrm,
    root: vrm.scene,
    neckBone: bone("neck"),
    headBone: headNode,
    spineBone: bone("upperChest") || bone("chest"),
    frame: { y: headPosition.y - VRM_FRAME_DROP, distance: VRM_FRAME_DISTANCE },
  };
}

function poseVrm(avatar, rig, delta, expressionWeights) {
  const turn = (node, share) => {
    if (node) node.quaternion.setFromEuler(new Euler(rig.pitch * share, rig.yaw * share, rig.roll * share));
  };
  turn(avatar.spineBone, 0.15);
  turn(avatar.neckBone, 0.35);
  turn(avatar.headBone, 0.5);
  const manager = avatar.vrm.expressionManager;
  if (manager) {
    for (const [name, value] of Object.entries(expressionWeights(rig))) {
      if (manager.getExpression(name)) manager.setValue(name, value);
    }
  }
  avatar.vrm.update(delta);
}

function lights(scene) {
  scene.add(new HemisphereLight(new Color("#dfe4ff"), new Color("#2a2140"), 1.6));
  const key = new DirectionalLight(new Color("#ffffff"), 2.2);
  key.position.set(-2, 3, 4);
  scene.add(key);
  const rim = new DirectionalLight(new Color("#ff9a8a"), 1.1);
  rim.position.set(3, 2, -3);
  scene.add(rim);
}

export async function createAvatarStage({ expressionWeights }) {
  const canvas = document.createElement("canvas");
  const renderer = new WebGLRenderer({ canvas, alpha: true, antialias: true, preserveDrawingBuffer: true });
  renderer.outputColorSpace = SRGBColorSpace;
  renderer.setClearColor(0x000000, 0);
  const scene = new Scene();
  lights(scene);
  const camera = new PerspectiveCamera(28, 16 / 9, 0.05, 50);
  const palette = makePalette();
  const builtins = new Map();
  let current = null;
  let custom = null;
  let lastSeconds = 0;

  function avatarFor(kind) {
    if (kind === "custom") return custom;
    if (!BUILDERS[kind]) return null;
    if (!builtins.has(kind)) builtins.set(kind, BUILDERS[kind](palette));
    return builtins.get(kind);
  }

  function show(avatar) {
    if (current === avatar) return;
    if (current) scene.remove(current.root);
    current = avatar;
    if (current) scene.add(current.root);
  }

  function aim(width, height, zoom) {
    if (canvas.width !== width || canvas.height !== height) renderer.setSize(width, height, false);
    camera.aspect = width / height;
    const { y, distance } = current.frame;
    const portrait = Math.max(1, (16 / 9) / camera.aspect);
    camera.position.set(0, y, distance * portrait * zoom);
    camera.lookAt(0, y, 0);
    camera.updateProjectionMatrix();
  }

  function draw(kind, rig, seconds, width, height, zoom = 1) {
    show(avatarFor(kind));
    if (!current) return null;
    const delta = Math.min(0.1, Math.max(0, seconds - lastSeconds));
    lastSeconds = seconds;
    if (current.kind === "vrm") poseVrm(current, rig, delta, expressionWeights);
    else poseBuiltin(current, rig, seconds);
    aim(width, height, zoom);
    renderer.render(scene, camera);
    return canvas;
  }

  async function setCustom(bytes) {
    const next = bytes && bytes.byteLength ? await loadVrm(bytes) : null;
    if (custom) {
      if (current === custom) show(null);
      VRMUtils.deepDispose(custom.root);
    }
    custom = next;
    return Boolean(custom);
  }

  function hasCustom() {
    return Boolean(custom);
  }

  function snapshot(kind, rig, size = 128) {
    const drawn = draw(kind, rig, 0, size, size, kind === "custom" ? 0.75 : 0.5);
    return drawn ? drawn.toDataURL("image/png") : null;
  }

  function dispose() {
    renderer.dispose();
  }

  return { draw, setCustom, hasCustom, snapshot, dispose };
}
