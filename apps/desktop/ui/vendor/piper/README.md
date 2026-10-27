# Piper text-to-speech

Built-in voices for /tts and Read Aloud run [Piper](https://github.com/rhasspy/piper) on the device: espeak-ng turns text into phonemes and ONNX Runtime turns phonemes into speech, both as WebAssembly inside the app window.

Only the small loaders live here. The engine binaries and the voices are downloaded the first time a built-in voice speaks, from the `tts-pack-1` release of this repository, and checked against the SHA-256 hashes in `apps/desktop/src-tauri/voice-pack.json` before use. The "Voice pack" workflow builds that release from the sources listed in the same file.

| File | Source | SHA-256 |
| --- | --- | --- |
| `ort.wasm.min.js` | npm `onnxruntime-web@1.20.1` | `e66568724f8848e57cc2a56e4bea3a5b86ce3ff81b2da11eac8ed7ab02b27bd4` |
| `ort-wasm-simd-threaded.mjs` | npm `onnxruntime-web@1.20.1` | `745eb7c0ce6f18a6aa521971b2877babc7ffb27eecb58ab3bc6e5ef4692672e8` |
| `piper_phonemize.js` | npm `@diffusionstudio/piper-wasm@1.0.0` | `fef0c2fc442d24fdef5c7c7cc37d5da2314407640fe11ab1bfe347c723dff19b` |

ONNX Runtime is licensed under the MIT License (`LICENSE-onnxruntime`). The phonemizer build is MIT licensed and includes espeak-ng, which is licensed under the GNU General Public License v3 or later. The voice models come from `rhasspy/piper-voices` (MIT); each voice's recordings carry the licence named as its `credit` in `voice-pack.json`. Alba and Siwis are trained on recordings from the University of Edinburgh's Centre for Speech Technology Research, used under CC BY 4.0.
