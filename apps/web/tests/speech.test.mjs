import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createRequire } from "node:module";
import { SPEECH_VOICE_ORIGINS, injectWebShell } from "../shell.mjs";

const require = createRequire(import.meta.url);
const { WEB_SPEECH_COMMANDS, WEB_VOICE_PACK_URL, webPackStatus, webPackFile, webVoicePack, webSpeechClip } = require("../ui/web-speech.js");
const { WEB_DEFAULTS } = require("../ui/web-commands.js");

const repo = join(import.meta.dirname, "..", "..", "..");
const read = (...path) => readFileSync(join(repo, ...path), "utf8");
const realPack = JSON.parse(read("apps", "desktop", "src-tauri", "voice-pack.json"));

function fileOf(name, bytes) {
  return { name, size: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex"), from: `https://cdn.example/${name}` };
}

function smallPack() {
  const bodies = { "rt.wasm": Buffer.from("runtime"), "v.onnx": Buffer.from("model bytes"), "v.onnx.json": Buffer.from("{}") };
  const pack = { release: "t", runtime: [fileOf("rt.wasm", bodies["rt.wasm"])], voices: [{ id: "v", name: "Vee", lang: "en-US", model: fileOf("v.onnx", bodies["v.onnx"]), config: fileOf("v.onnx.json", bodies["v.onnx.json"]) }] };
  return { pack, bodies };
}

function fakeWindow(pack, bodies, broken = {}) {
  const store = new Map();
  const fetched = [];
  const win = {
    fetched,
    store,
    location: { origin: "https://commy.chat" },
    crypto: globalThis.crypto,
    Response,
    async fetch(url) {
      fetched.push(url);
      if (url === WEB_VOICE_PACK_URL) return new Response(JSON.stringify(pack));
      const name = url.split("/").pop();
      if (broken[name]) return new Response(broken[name]);
      return new Response(bodies[name]);
    },
    caches: {
      async open() {
        return {
          async keys() { return [...store.keys()].map(url => ({ url })); },
          async match(request) { const hit = store.get(typeof request === "string" ? request : request.url); return hit ? hit.clone() : undefined; },
          async put(url, response) { store.set(url, response); },
          async delete(url) { return store.delete(url); },
        };
      },
    },
  };
  return win;
}

test("a built-in voice is offered in the browser and says how much it still needs to download", async () => {
  const { pack, bodies } = smallPack();
  const events = [];
  const voices = webVoicePack(fakeWindow(pack, bodies), (event, payload) => events.push([event, JSON.parse(payload)]));
  const before = await voices.status();
  assert.deepEqual(before, { available: true, voices: [{ id: "v", name: "Vee", lang: "en-US", ready: false, download_bytes: 7 + 11 + 2 }] });
  await voices.download({ voice: "v" });
  assert.deepEqual((await voices.status()).voices[0], { id: "v", name: "Vee", lang: "en-US", ready: true, download_bytes: 0 });
  assert.deepEqual(events.at(-1), ["voice-pack-progress", { voice: "v", loaded: 20, total: 20 }]);
  assert.equal(Buffer.from(await voices.file({ name: "v.onnx" })).toString(), "model bytes");
});

test("a voice download that does not match its hash is never kept", async () => {
  const { pack, bodies } = smallPack();
  const win = fakeWindow(pack, bodies, { "v.onnx": Buffer.from("model bytez") });
  const voices = webVoicePack(win, () => {});
  await assert.rejects(voices.download({ voice: "v" }), /not the expected file/);
  assert.equal([...win.store.keys()].some(url => url.endsWith("v.onnx")), false);
  await assert.rejects(voices.file({ name: "v.onnx" }), /not downloaded/);
  await assert.rejects(voices.file({ name: "elsewhere.bin" }), /unknown voice file/);
});

test("a browser without storage for voices offers none, so the app uses the device's voices", async () => {
  const { pack, bodies } = smallPack();
  const win = fakeWindow(pack, bodies);
  delete win.caches;
  const status = await webVoicePack(win, () => {}).status();
  assert.equal(status.available, false);
  assert.equal(status.voices[0].ready, false);
  assert.deepEqual(webPackStatus(null, new Map(), true), { available: false, voices: [] });
});

test("the browser fetches each voice file only from a site its page policy allows", () => {
  const files = [...realPack.runtime, ...realPack.voices.flatMap(v => [v.model, v.config])];
  assert.ok(files.length >= 5);
  for (const file of files) {
    const host = new URL(file.from).origin;
    assert.ok(SPEECH_VOICE_ORIGINS.includes(host), file.from);
    assert.equal(webPackFile(realPack, file.name), file);
  }
  assert.ok(realPack.voices.some(v => v.id === "en_US-kristin-medium"));
});

test("speech audio from the app becomes samples at its rate, queued or starting over", () => {
  const samples = new Float32Array([0.25, -0.5]);
  const clip = webSpeechClip(new Uint8Array(samples.buffer), { headers: { "x-commy-rate": "22050", "x-commy-append": "1" } });
  assert.deepEqual(Array.from(clip.samples), [0.25, -0.5]);
  assert.equal(clip.rate, 22050);
  assert.equal(clip.append, true);
  assert.equal(webSpeechClip(new Uint8Array(8), { headers: { "x-commy-rate": "16000", "x-commy-append": "0" } }).append, false);
  assert.throws(() => webSpeechClip(new Uint8Array(8), { headers: {} }));
});

test("the built-in voices run in the browser like the desktop: same commands, engine and pack", () => {
  const app = read("apps", "desktop", "ui", "scripts", "app.js");
  for (const cmd of WEB_SPEECH_COMMANDS) {
    assert.ok(app.includes(`invoke("${cmd}"`), cmd);
    assert.equal(Object.prototype.hasOwnProperty.call(WEB_DEFAULTS, cmd), false, cmd);
  }
  const build = read("apps", "web", "build.mjs");
  assert.match(build, /"web-speech\.js", "scripts\/web-speech\.js"/);
  assert.match(build, /voice-pack\.json/);
  assert.doesNotMatch(build, /"apps\/desktop\/ui\/vendor\/"/);
  assert.ok(app.includes('loadVendorScript("vendor/piper/'));
  const html = injectWebShell(read("apps", "desktop", "ui", "index.html"), { relays: ["https://relay.commy.chat:8443"] });
  const speech = html.indexOf("scripts/web-speech.js");
  assert.ok(html.indexOf("scripts/web-voice.js") < speech && speech < html.indexOf("scripts/web-bridge.js"));
  assert.match(html, /connect-src 'self' blob: /);
  assert.ok(read("apps", "web", "ui", "web-bridge.js").includes("WEB_SPEECH_COMMANDS.includes(cmd)"));
});
