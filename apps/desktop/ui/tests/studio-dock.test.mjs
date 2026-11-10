import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { STUDIO_DOCK_BARS, STUDIO_DOCK_FUSE_MS, STUDIO_DOCK_LINES, studioDockQuality, studioDockWords, studioDockDesk, mountStudioDock, studioDockSound, createStudioDockFuse } = require("../scripts/studio-dock.js");
const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const css = read("../styles/studio.css");
const app = read("../scripts/app.js");

function fakeNode(tag) {
  return {
    tag,
    id: "",
    className: "",
    title: "",
    attrs: {},
    children: [],
    classes: new Set(),
    setAttribute(name, value) { this.attrs[name] = value; },
    appendChild(child) { this.children.push(child); return child; },
    append(...kids) { this.children.push(...kids); },
    prepend(...kids) { this.children.unshift(...kids); },
    querySelector(selector) { return this.children.find(child => selector === `.${child.className.split(" ")[1]}`) || null; },
    classList: null,
  };
}

function fakeDock(look, fit, withRow = true) {
  const row = fakeNode("div");
  const panel = fakeNode("section");
  const toggled = new Set();
  panel.classList = { toggle(name, on) { if (on) toggled.add(name); else toggled.delete(name); }, contains: name => toggled.has(name) };
  panel.querySelector = selector => (selector === ".vp-voice" && withRow ? row : null);
  return {
    row,
    panel,
    documentElement: { dataset: { ...(look ? { look } : {}), ...(fit ? { fit } : {}) } },
    createElement: fakeNode,
    getElementById: id => (id === "voice-panel" ? panel : null),
  };
}

test("the dock says the state on line one and keeps the room on line two until you mute", () => {
  const live = studioDockWords({ status: "Voice Connected", where: "Lounge / Pixel Pioneers", muted: false, deafened: false, armed: false });
  assert.deepEqual(live, { state: "live", line: "Voice Connected", sub: "Lounge / Pixel Pioneers" });
  const muted = studioDockWords({ status: "Voice Connected", where: "Lounge / Pixel Pioneers", muted: true });
  assert.deepEqual(muted, { state: "muted", line: "Muted", sub: STUDIO_DOCK_LINES.muted[1] });
  assert.equal(studioDockWords({ muted: true, deafened: true }).state, "deafened");
  assert.equal(studioDockWords({ muted: true, deafened: true }).line, "Deafened");
  const armed = studioDockWords({ status: "Voice Connected", where: "Lounge", muted: true, deafened: true, armed: true });
  assert.deepEqual(armed, { state: "armed", line: "Press again to leave", sub: "Or press anything else to stay." });
  assert.deepEqual(studioDockWords({}), { state: "live", line: "", sub: "" });
});

test("connection quality has words for all three levels and falls back to good", () => {
  assert.equal(studioDockQuality("good").line, "Good connection");
  assert.equal(studioDockQuality("ok").line, "Fair connection");
  assert.equal(studioDockQuality("bad").line, "Poor connection");
  assert.deepEqual(studioDockQuality("nonsense"), studioDockQuality("good"));
  for (const level of ["good", "ok", "bad"]) assert.ok(studioDockQuality(level).sub.length > 0);
});

test("the desktop dock is the new look outside the phone fit", () => {
  assert.equal(studioDockDesk(fakeDock("studio", "roomy")), true);
  assert.equal(studioDockDesk(fakeDock("studio", "compact")), true);
  assert.equal(studioDockDesk(fakeDock("studio", "slim")), false);
  assert.equal(studioDockDesk(fakeDock(null, "roomy")), false);
});

test("the invite key, the hand key, the level meter and the fuse are built once and only in the new look", () => {
  assert.equal(mountStudioDock(fakeDock(null, "roomy"), null), null);
  assert.equal(mountStudioDock(fakeDock("studio", "roomy", false), null), null);
  const doc = fakeDock("studio", "roomy");
  const icons = [];
  const invite = mountStudioDock(doc, (name, size) => {
    icons.push([name, size]);
    return fakeNode("svg");
  });
  assert.equal(invite.id, "vp-invite");
  assert.equal(invite.attrs["aria-label"], "Invite");
  assert.deepEqual(icons, [["hand", 18], ["user-plus", 18]]);
  assert.deepEqual(doc.row.children.map(child => child.className), ["icon-btn vp-hand-key hidden", "icon-btn vp-invite hidden", "vp-meter"]);
  const meter = doc.row.children[2];
  assert.equal(meter.children.length, STUDIO_DOCK_BARS);
  assert.equal(meter.attrs["aria-hidden"], "true");
  assert.equal(doc.panel.children[0].className, "vp-fuse");
  assert.equal(mountStudioDock(doc, null), invite);
  assert.equal(doc.row.children.length, 3);
});

test("the meter only moves while somebody in the room is heard", () => {
  const doc = fakeDock("studio", "roomy");
  assert.equal(studioDockSound(doc, true), true);
  assert.equal(doc.panel.classList.contains("has-sound"), true);
  studioDockSound(doc, 0);
  assert.equal(doc.panel.classList.contains("has-sound"), false);
  assert.equal(studioDockSound({ getElementById: () => null }, true), false);
  assert.match(css, /\.voice-panel\.has-sound \.vp-meter i \{[^}]*animation: studio-meter/);
  assert.match(css, /\.vp-meter i \{[^}]*scale: 1 \.12;/);
  assert.match(css, /\[data-dock="deafened"\] \.vp-meter i \{[^}]*animation: none;/);
});

test("hang up asks twice and the first press wears off", () => {
  const timers = [];
  const seen = [];
  const fuse = createStudioDockFuse(STUDIO_DOCK_FUSE_MS, { set: (fn, ms) => timers.push({ fn, ms }) - 1, clear: id => { timers[id].cleared = true; } });
  fuse.watch(on => seen.push(on));
  assert.equal(fuse.armed(), false);
  assert.equal(fuse.cancel(), false);
  assert.equal(fuse.press(), "armed");
  assert.equal(fuse.armed(), true);
  assert.equal(timers[0].ms, 3000);
  assert.equal(fuse.press(), "leave");
  assert.equal(timers[0].cleared, true);
  assert.equal(fuse.armed(), false);
  assert.equal(fuse.press(), "armed");
  timers[1].fn();
  assert.equal(fuse.armed(), false);
  assert.equal(fuse.press(), "armed");
  assert.equal(fuse.cancel(), true);
  assert.equal(fuse.armed(), false);
  assert.deepEqual(seen, [true, false, true, false, true, false]);
});

test("the dock script loads after the icons and before the app, and the page has the voice pop-up", () => {
  const icons = html.indexOf('<script src="scripts/studio-icons.js"></script>');
  const dock = html.indexOf('<script src="scripts/studio-dock.js"></script>');
  const main = html.indexOf('<script src="scripts/app.js"></script>');
  assert.ok(icons > 0 && dock > icons && main > dock);
  for (const id of ["voice-pop", "vop-room", "vop-quality", "vop-mic", "vop-speakers", "vop-camera", "chm-activities"]) assert.ok(html.includes(`id="${id}"`), id);
  assert.match(html, /<div class="voice-pop hidden" id="voice-pop"/);
});

test("in the new look the dock holds only what the guide draws", () => {
  for (const part of [".vp-topic", ".vp-grid", ".vp-streams", ".vp-stage", ".vp-talk", ".vp-game-share", "#vp-activities", ".caret-btn"]) {
    assert.ok(css.includes(`html[data-look="studio"]:not([data-fit="slim"]) .voice-panel ${part}`), part);
  }
  assert.match(app, /if \(studioDesk\(\) && channel\.id === state\.voiceChannelId\) \{\n\s+showCallGrid\(\);/);
  assert.match(app, /\$\("chm-activities"\)\.onclick/);
  assert.match(app, /if \(!studioDesk\(\) \|\| studioHang\.press\(\) === "leave"\) disconnectVoice\(\);/);
});

test("the signal mark is five bars, and fair and poor light three and one", () => {
  const dock = require("../scripts/studio-dock.js");
  assert.equal(dock.STUDIO_DOCK_SIGNAL_BARS.length, 5);
  assert.deepEqual(dock.STUDIO_DOCK_SIGNAL_LIT, { good: 5, ok: 3, bad: 1 });
  const markup = dock.studioDockSignalMarkup();
  assert.equal(markup.match(/<path /g).length, 5);
  assert.ok(markup.startsWith('<svg class="i ai vp-bars"'));
  const desk = 'html[data-look="studio"]:not([data-fit="slim"])';
  assert.ok(css.includes(`${desk} .vp-signal[data-quality="ok"] .vp-bars path:nth-child(n+${dock.STUDIO_DOCK_SIGNAL_LIT.ok + 1}),`));
  assert.ok(css.includes(`${desk} .vp-signal[data-quality="bad"] .vp-bars path:nth-child(n+${dock.STUDIO_DOCK_SIGNAL_LIT.bad + 1}),`));
  assert.ok(app.includes('$("vp-signal").replaceChildren(studioLook() ? studioDockSignal(document, 14) : svgIcon("signal", 14));'));
});

test("a key that is on is lit glass with a green lamp, never the switched-off grey", () => {
  const desk = 'html[data-look="studio"]:not([data-fit="slim"])';
  const lit = css.slice(css.indexOf(`${desk} .vp-activities.vp-share.active,`));
  const block = lit.slice(0, lit.indexOf("}") + 1);
  assert.ok(block.includes("#vp-camera.active"));
  assert.ok(block.includes(".vp-hand-key.on"));
  assert.ok(block.includes("color: var(--header-primary);"));
  assert.ok(!block.includes("--mute-k"));
  assert.ok(css.includes(`${desk} .vp-activities.vp-share.active::after {`));
  assert.ok(css.includes("background: var(--glass-hi), var(--glass-key-on);"));
});
