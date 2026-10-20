# MediaPipe Tasks Vision

Background blur and custom background images use the person segmenter from MediaPipe Tasks Vision, and camera avatars follow your face with its face landmarker, bundled so it runs on the device with no network requests.

| File | Source | SHA-256 |
| --- | --- | --- |
| `vision_bundle.mjs` | npm `@mediapipe/tasks-vision@0.10.35` | `55d7ab624fbb70dcc5adc4ae6d7ea9cfcb569139d3dbfbf2b1deafcb966bc0fe` |
| `wasm/vision_wasm_internal.js` | npm `@mediapipe/tasks-vision@0.10.35` | `e7fd9858e8e8f221d9b96eddc11f8e077f263e0b7bbd79d3cbe882b134274f8c` |
| `wasm/vision_wasm_internal.wasm` | npm `@mediapipe/tasks-vision@0.10.35` | `6a5c64584c2ab61c763b6e204afbdbc7ce1caf7f5216187322bca8df94f646bc` |
| `face_landmarker.task` | `storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/latest/` | `64184e229b263107bc2b804c6625db1341ff2bb731874b0bcc2fe6544e0bc9ff` |
| `selfie_segmenter.tflite` | `storage.googleapis.com/mediapipe-models/image_segmenter/selfie_segmenter/float16/latest/` | `191ac9529ae506ee0beefa6b2c945a172dab9d07d1e802a290a4e4038226658b` |

Licensed under the Apache License 2.0 (`LICENSE`).

Stay on 0.10.x when upgrading: the 1.x packages send usage logs to a Google endpoint. `tests/camera-fx.test.mjs` fails if a bundled file contains that endpoint.
