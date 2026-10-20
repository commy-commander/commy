# three.js and three-vrm

Camera avatars are drawn with three.js and load VRM avatar files with three-vrm. Both are bundled into one file so they run on the device with no network requests, and avatar files may only load textures embedded in the file itself.

| File | Source | SHA-256 |
| --- | --- | --- |
| `three_bundle.mjs` | npm `three@0.186.1` and `@pixiv/three-vrm@3.5.5`, built with `esbuild@0.25.11` | `59e4d6491aa6163a367f45ddba81c40a85f5e04b09168371de5203d7d51a92e8` |

Licensed under the MIT License (`LICENSE`).

To rebuild, install those three packages in an empty folder, save this as `entry.mjs`, and run `npx esbuild entry.mjs --bundle --format=esm --minify --legal-comments=inline --target=es2020 --outfile=three_bundle.mjs`:

```js
export {
  WebGLRenderer, Scene, PerspectiveCamera, Group, Mesh, SphereGeometry, CylinderGeometry, ConeGeometry,
  CapsuleGeometry, CircleGeometry, MeshToonMaterial, MeshBasicMaterial, DataTexture, RedFormat,
  NearestFilter, HemisphereLight, DirectionalLight, Color, Vector3, Euler, BackSide, SRGBColorSpace, LoadingManager,
} from "three";
export { RoundedBoxGeometry } from "three/examples/jsm/geometries/RoundedBoxGeometry.js";
export { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
export { VRMLoaderPlugin, VRMUtils } from "@pixiv/three-vrm";
```
