import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { createRequire } from "node:module";
import { contentSecurityPolicy, injectWebShell, relayOrigins, webRelayList } from "../shell.mjs";

const require = createRequire(import.meta.url);
const { WEB_BYTE_COMMANDS, WEB_PICKED_COMMANDS, WEB_VIDEO_ACCEPT, webPickFile, webSaveFile, WEB_DEFAULTS, webDefault, webFileSrc, webSignInUrl, webUnlockText, webReplyLine, webRelays, webNotice, webNoticeTarget } = require("../ui/web-commands.js");
const webVoiceScripts = require("../ui/web-voice.js");
const worklet = require("../ui/web-voice-worklet.js");

const repo = join(import.meta.dirname, "..", "..", "..");
const read = (...path) => readFileSync(join(repo, ...path), "utf8");
const rustCommands = new Set([...read("apps", "web", "src", "session.rs").matchAll(/^\s+"([a-z_]+)" =>/gm)].map(m => m[1]));

test("every command the app calls while starting up has an answer in the browser", () => {
  const boot = /async function boot\(\) \{([\s\S]*?)\n\}/.exec(read("apps", "desktop", "ui", "scripts", "app.js"))[1];
  const loaders = [...boot.matchAll(/await (load[A-Za-z]+)\(\)/g)].map(m => m[1]);
  const app = read("apps", "desktop", "ui", "scripts", "app.js");
  const called = new Set(["get_voice_settings", "get_user", "list_servers", "get_server", "get_messages", "send_message"]);
  for (const loader of loaders) {
    const body = new RegExp(`function ${loader}\\(\\) \\{([\\s\\S]*?)\\n\\}`).exec(app)[1];
    for (const [, cmd] of body.matchAll(/invoke\("([a-z_]+)"/g)) called.add(cmd);
  }
  const missing = [...called].filter(cmd => !webDefault(cmd).known && !rustCommands.has(cmd));
  assert.deepEqual(missing, []);
});

test("the browser answers each command once, either from defaults or from the backend", () => {
  const both = Object.keys(WEB_DEFAULTS).filter(cmd => rustCommands.has(cmd));
  assert.deepEqual(both, []);
});

test("a notice is clipped like the desktop's and opens only a channel or DM", () => {
  const id = "0f3c2a9e-1b2c-4d5e-8f90-123456789abc";
  const notice = webNotice({ title: "é".repeat(100), body: "x".repeat(300), target: `channel:${id}:${id}` });
  assert.equal(Array.from(notice.title).length, 80);
  assert.equal(notice.options.body.length, 240);
  assert.equal(notice.options.tag, `channel:${id}:${id}`);
  assert.equal(notice.target, `channel:${id}:${id}`);
  assert.equal(webNoticeTarget(`dm:${id}`), `dm:${id}`);
  assert.equal(webNoticeTarget("https://example.com"), null);
  assert.equal(webNoticeTarget(`dm:${id}:extra`), null);
  const plain = webNotice({ title: "", body: "hi" });
  assert.equal(plain.title, "Commy");
  assert.equal(plain.options.tag, undefined);
  assert.equal(plain.target, null);
});

test("notices go to the browser rather than being dropped", () => {
  assert.equal(webDefault("notify").known, false);
  assert.match(read("apps", "web", "ui", "web-bridge.js"), /cmd === "notify"/);
});

test("defaults are fresh copies, so the app cannot change them for the next caller", () => {
  const first = webDefault("list_audio_devices").value;
  first.inputs.push({ id: "x" });
  assert.deepEqual(webDefault("list_audio_devices").value.inputs, []);
  assert.equal(webDefault("create_server").known, false);
});

test("sign-in goes to the same site's account page", () => {
  assert.equal(webSignInUrl("https://commy.chat"), "https://commy.chat/account");
});

test("sign-in brings the visitor back to the web app's own path", () => {
  assert.equal(webSignInUrl("https://commy.chat", "/web/"), "https://commy.chat/account?next=%2Fweb%2F");
});

test("pictures from GIF and emoji search load through this site's account server", () => {
  assert.equal(webFileSrc("", "commy-media"), "/api/auth/commy/media");
  assert.equal(webFileSrc("blob:x", "asset"), "blob:x");
});

test("the browser's sign-in text is not replaced by the desktop's translations", () => {
  assert.match(read("apps", "web", "ui", "web-bridge.js"), /removeAttribute\("data-i18n"\)/);
});

test("the unlock text prefers the backend's detail", () => {
  assert.equal(webUnlockText({ state: "locked", detail: "Try again" }), "Try again");
  assert.match(webUnlockText({ state: "no_list", detail: "" }), /computer/);
  assert.match(webUnlockText(null), /recovery code/);
});

test("replies cross into the backend as a status line and body", () => {
  assert.equal(webReplyLine(200, '{"a":1}'), '200\n{"a":1}');
});

test("the page may only talk to its own site and the relay, and loads nothing from elsewhere", () => {
  const csp = contentSecurityPolicy(relayOrigins("https://relay.commy.chat:8443"));
  assert.match(csp, /default-src 'self'/);
  assert.match(csp, /connect-src 'self' https:\/\/relay\.commy\.chat:8443 wss:\/\/relay\.commy\.chat:8443;/);
  assert.match(csp, /frame-src 'none'/);
  assert.doesNotMatch(csp, /\*/);
});

test("the bridge loads before the app and the policy lands in the head", () => {
  const html = injectWebShell(read("apps", "desktop", "ui", "index.html"), { relays: ["https://relay.commy.chat:8443"] });
  const bridge = html.indexOf("scripts/web-bridge.js");
  const commands = html.indexOf("scripts/web-commands.js");
  assert.ok(commands > 0 && commands < bridge && bridge < html.indexOf('src="scripts/app.js"'));
  assert.ok(html.indexOf("Content-Security-Policy") < html.indexOf("</head>"));
  assert.ok(html.includes('href="styles/web.css"'));
});

test("the bridge keeps its helpers private, so it cannot replace a function the app declares", () => {
  const source = read("apps", "web", "ui", "web-bridge.js");
  assert.match(source, /^\(\(\) => \{/m);
  assert.doesNotMatch(source, /^(?:async\s+)?function\s/m);
});

test("a local test relay is reached over plain http and ws", () => {
  assert.deepEqual(relayOrigins("http://localhost:3340"), ["http://localhost:3340", "ws://localhost:3340"]);
});

test("a page served under a path gets a base, so its relative scripts and styles still load", () => {
  const index = read("apps", "desktop", "ui", "index.html");
  const html = injectWebShell(index, { relays: ["https://relay.commy.chat:8443"], base: "/web/" });
  const baseAt = html.indexOf('<base href="/web/" />');
  assert.ok(baseAt > html.indexOf("<head>"));
  assert.ok(baseAt < html.indexOf('rel="icon"') && baseAt < html.indexOf('href="styles/styles.css"'));
  assert.doesNotMatch(injectWebShell(index, { relays: ["https://relay.commy.chat:8443"] }), /<base /);
});

test("the page allows every relay it may be pointed at and names the first as its relay", () => {
  const relays = webRelayList(" https://relay.commy.chat:8443/, https://relay-staging.commy.chat:8443 ", "https://fallback.example");
  assert.deepEqual(relays, ["https://relay.commy.chat:8443", "https://relay-staging.commy.chat:8443"]);
  const html = injectWebShell(read("apps", "desktop", "ui", "index.html"), { relays });
  assert.match(html, /connect-src 'self' https:\/\/relay\.commy\.chat:8443 wss:\/\/relay\.commy\.chat:8443 https:\/\/relay-staging\.commy\.chat:8443 wss:\/\/relay-staging\.commy\.chat:8443;/);
  assert.match(html, /<meta name="commy-relays" content="https:\/\/relay\.commy\.chat:8443" \/>/);
  assert.deepEqual(webRelayList("", "https://fallback.example"), ["https://fallback.example"]);
});

test("the backend gets the relay the page names, or its built-in one when the page names none", () => {
  const page = content => ({ querySelector: () => (content === null ? null : { getAttribute: () => content }) });
  assert.equal(webRelays(page("https://relay-staging.commy.chat:8443")), "https://relay-staging.commy.chat:8443");
  assert.equal(webRelays(page(" ")), undefined);
  assert.equal(webRelays(page(null)), undefined);
});

test("every image the bridge shows ships with the app", () => {
  const images = [...read("apps", "web", "ui", "web-bridge.js").matchAll(/src="(images\/[^"]+)"/g)].map(m => m[1]);
  assert.ok(images.length > 0);
  for (const image of images) assert.ok(existsSync(join(repo, "apps", "desktop", "ui", image)), image);
});

test("the voice engine loads before the bridge that hands it to the backend", () => {
  const html = injectWebShell(read("apps", "desktop", "ui", "index.html"), { relays: ["https://relay.commy.chat:8443"] });
  const voice = html.indexOf("scripts/web-voice.js");
  assert.ok(html.indexOf("scripts/web-commands.js") < voice && voice < html.indexOf("scripts/web-bridge.js"));
  assert.match(read("apps", "web", "build.mjs"), /"web-voice-worklet\.js", "scripts\/web-voice-worklet\.js"/);
  assert.equal(webVoiceScripts.WEB_VOICE_WORKLET, "scripts/web-voice-worklet.js");
});

test("the microphone is asked for with the browser's echo, noise and level handling on", () => {
  assert.deepEqual(webVoiceScripts.webMicConstraints().audio, { echoCancellation: true, noiseSuppression: true, autoGainControl: true, channelCount: 1 });
});

function fakeVoiceWindow() {
  const node = () => ({ port: {}, connect(next) { return next; }, disconnect() {} });
  const win = {
    asked: 0,
    tracks: [],
    timers: [],
    setTimeout(callback) { win.timers.push(callback); },
    AudioContext: class { constructor() { this.state = "running"; this.audioWorklet = { addModule: async () => {} }; this.destination = node(); } createMediaStreamSource() { return node(); } createGain() { return { ...node(), gain: {} }; } },
    AudioWorkletNode: class { constructor() { Object.assign(this, node()); } },
    navigator: {
      mediaDevices: {
        async getUserMedia() {
          win.asked += 1;
          const track = { stopped: false, stop() { this.stopped = true; } };
          win.tracks.push(track);
          return { getTracks: () => [track], getAudioTracks: () => [track] };
        },
      },
    },
  };
  return win;
}

test("a microphone that disappears mid-call, like a headset muting itself, is opened again", async () => {
  const win = fakeVoiceWindow();
  const voice = webVoiceScripts.webVoice(win);
  assert.equal(await voice.start(() => {}), true);
  assert.equal(win.asked, 1);
  win.tracks[0].onended();
  assert.equal(win.tracks[0].stopped, true);
  await win.timers.shift()();
  assert.equal(win.asked, 2);
  voice.stop();
  assert.equal(win.tracks[1].stopped, true);
  assert.equal(win.tracks[1].onended, null);
});

test("a microphone that ends after the call is over is left alone", async () => {
  const win = fakeVoiceWindow();
  const voice = webVoiceScripts.webVoice(win);
  await voice.start(() => {});
  const ended = win.tracks[0].onended;
  voice.stop();
  ended();
  assert.equal(win.timers.length, 0);
  assert.equal(win.asked, 1);
});

test("the microphone is cut into 10 ms frames at 48 kHz whatever rate the browser records at", () => {
  for (const rate of [48000, 44100, 16000]) {
    const frames = [];
    const framer = new worklet.MicFramer(rate, frame => frames.push(frame));
    const block = new Float32Array(128).fill(0.5);
    for (let i = 0; i < rate / 128; i++) framer.push(block);
    assert.ok(Math.abs(frames.length - 100) <= 1, `${rate}: ${frames.length}`);
    assert.ok(frames.every(frame => frame.length === worklet.VOICE_FRAME));
    assert.ok(frames.every(frame => frame.every(sample => Math.abs(sample - 16384) <= 1)));
  }
});

test("loud input is clipped, not wrapped", () => {
  const frames = [];
  const framer = new worklet.MicFramer(48000, frame => frames.push(frame));
  framer.push(new Float32Array(480).fill(3));
  framer.push(new Float32Array(480).fill(-3));
  assert.equal(frames[0][0], 32767);
  assert.equal(frames[1][0], -32767);
});

test("the speaker waits for a short cushion, then plays what arrives in order", () => {
  const queue = new worklet.SpeakerQueue(48000);
  const out = new Float32Array(128);
  queue.add(Int16Array.from({ length: 480 }, (_, i) => i));
  queue.fill(out);
  assert.ok(out.every(sample => sample === 0));
  for (let n = 1; n * 480 < worklet.SPEAKER_START; n++) queue.add(Int16Array.from({ length: 480 }, (_, i) => n * 480 + i));
  queue.fill(out);
  for (let i = 1; i < out.length; i++) assert.ok(Math.abs(out[i] * 32768 - (i - 1)) < 1e-6, `${i}: ${out[i] * 32768}`);
});

test("the speaker drops the oldest sound when it falls far behind, and goes quiet when it runs dry", () => {
  const queue = new worklet.SpeakerQueue(48000);
  for (let n = 0; n < 100; n++) queue.add(new Int16Array(480).fill(n));
  assert.ok(queue.queued <= worklet.SPEAKER_MOST);
  const out = new Float32Array(128);
  queue.fill(out);
  assert.ok(out[127] * 32768 > 60);
  for (let i = 0; i < 200; i++) queue.fill(out);
  assert.ok(out.every(sample => sample === 0));
  assert.equal(queue.playing, false);
});

function fakeDocument() {
  const made = [];
  const body = { children: [], appendChild(node) { this.children.push(node); } };
  const doc = {
    body,
    made,
    createElement(tag) {
      const listeners = {};
      const node = {
        tag,
        clicks: 0,
        addEventListener(event, callback) { listeners[event] = callback; },
        fire(event) { listeners[event](); },
        click() { this.clicks += 1; },
        remove() { body.children = body.children.filter(child => child !== node); },
      };
      made.push(node);
      return node;
    },
  };
  return doc;
}

test("file commands the browser answers with bytes are handled by the backend", () => {
  for (const cmd of [...WEB_BYTE_COMMANDS, "attach_dm_bytes", "get_attachment_preview"]) {
    assert.equal(webDefault(cmd).known, false, cmd);
  }
  for (const cmd of ["attach_dm_bytes", "get_attachment_preview"]) assert.ok(rustCommands.has(cmd), cmd);
});

test("picked files and the media the browser fetches itself are never answered by a default", () => {
  for (const cmd of [...WEB_PICKED_COMMANDS, "attach_bytes", "get_profile_image", "fetch_server_image", "fetch_expression", "start_voice_recording", "cancel_voice_recording", "send_voice_message", "send_dm_voice_message"]) {
    assert.equal(webDefault(cmd).known, false, cmd);
  }
  for (const cmd of ["attach_bytes", "get_profile_image", "fetch_server_image", "fetch_expression", "start_voice_recording", "cancel_voice_recording", "send_voice_message", "send_dm_voice_message"]) {
    assert.ok(rustCommands.has(cmd), cmd);
  }
  assert.ok(WEB_PICKED_COMMANDS.includes("attach_file") && WEB_PICKED_COMMANDS.includes("attach_dm_file"));
});

test("picking a file opens the browser's own picker and gives back the chosen file", async () => {
  const doc = fakeDocument();
  const picking = webPickFile(doc, "video");
  const [input] = doc.made;
  assert.equal(input.type, "file");
  assert.equal(input.accept, WEB_VIDEO_ACCEPT);
  assert.equal(input.clicks, 1);
  input.files = [{ name: "clip.mp4" }];
  input.fire("change");
  assert.deepEqual(await picking, { name: "clip.mp4" });
});

test("closing the picker sends nothing", async () => {
  const doc = fakeDocument();
  const picking = webPickFile(doc);
  assert.equal(doc.made[0].accept, undefined);
  doc.made[0].fire("cancel");
  assert.equal(await picking, null);
});

test("a downloaded file is saved under its own name and leaves nothing behind on the page", () => {
  const doc = fakeDocument();
  const revoked = [];
  const urls = { createObjectURL: () => "blob:one", revokeObjectURL: url => revoked.push(url) };
  const saved = webSaveFile(doc, urls, new Uint8Array([1, 2, 3]), "report.pdf");
  const [link] = doc.made;
  assert.equal(saved, "report.pdf");
  assert.equal(link.href, "blob:one");
  assert.equal(link.clicks, 1);
  assert.deepEqual(doc.body.children, []);
  assert.equal(webSaveFile(fakeDocument(), urls, new Uint8Array([1]), ""), "file");
});
