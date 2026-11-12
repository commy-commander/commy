import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const {
  NOT_CONNECTED,
  audioDeviceIdentity, sameAudioDevice, connectedAudioDevice, audioDevicePicks,
  readCameraChoice, cameraChoiceText, findCamera, cameraPicks, cameraChoiceFor, cameraMove,
} = require("../scripts/device-choice.js");

const ui = join(dirname(fileURLToPath(import.meta.url)), "..");
const webcam = { deviceId: "cam-usb", label: "USB Webcam" };
const laptop = { deviceId: "cam-built-in", label: "Built-in Camera" };

test("a mic or speaker the system renumbers is still the one you chose", () => {
  assert.equal(audioDeviceIdentity("Headset (2- USB Audio)"), "headset (usb audio)");
  assert.ok(sameAudioDevice("Headset (USB Audio)", "Headset (3- USB Audio)"));
  assert.ok(!sameAudioDevice("Headset (USB Audio)", "Speakers (USB Audio)"));
  assert.ok(!sameAudioDevice("Line (2-channel)", "Line (channel)"));
  assert.ok(!sameAudioDevice("", "Headset"));
  assert.equal(connectedAudioDevice(["Speakers", "Headset (2- USB Audio)"], "Headset (USB Audio)"), "Headset (2- USB Audio)");
  assert.equal(connectedAudioDevice(["Headset (2- USB Audio)", "Headset (USB Audio)"], "Headset (USB Audio)"), "Headset (USB Audio)");
  assert.equal(connectedAudioDevice(["Speakers"], "Headset"), null);
});

test("a chosen mic that is unplugged stays chosen in the picker", () => {
  const picks = audioDevicePicks(["Speakers"], "Headset");
  assert.equal(picks.value, "Headset");
  assert.deepEqual(picks.options.at(-1), { value: "Headset", label: `Headset ${NOT_CONNECTED}` });
  assert.deepEqual(audioDevicePicks(["Speakers", "Headset"], "Headset").options.map(o => o.value), ["", "Speakers", "Headset"]);
  assert.equal(audioDevicePicks(["Speakers", "Headset (2- USB)"], "Headset (USB)").value, "Headset (2- USB)");
  assert.equal(audioDevicePicks(["Speakers"], null).value, "");
  assert.equal(audioDevicePicks(["Speakers"], "  ").options.length, 2);
});

test("the saved camera keeps its name and reads the older saved form", () => {
  assert.deepEqual(readCameraChoice(cameraChoiceText({ id: "cam-usb", label: "USB Webcam" })), { id: "cam-usb", label: "USB Webcam" });
  assert.deepEqual(readCameraChoice("cam-usb"), { id: "cam-usb", label: "" });
  assert.equal(readCameraChoice(""), null);
  assert.equal(readCameraChoice(null), null);
  assert.equal(readCameraChoice("{broken"), null);
  assert.equal(readCameraChoice(JSON.stringify({ id: "" })), null);
  assert.equal(cameraChoiceText(null), "");
});

test("a camera is found by its id, or by its name when the id changed", () => {
  assert.equal(findCamera([laptop, webcam], { id: "cam-usb", label: "" }), webcam);
  assert.equal(findCamera([laptop, { deviceId: "cam-new", label: "USB Webcam" }], { id: "cam-usb", label: "USB Webcam" }).deviceId, "cam-new");
  assert.equal(findCamera([laptop], { id: "cam-usb", label: "USB Webcam" }), null);
  assert.equal(findCamera([{ deviceId: "", label: "" }], { id: "", label: "" }), null);
  assert.equal(findCamera([laptop], null), null);
});

test("an unplugged camera stays chosen in the picker", () => {
  const picks = cameraPicks([laptop], { id: "cam-usb", label: "USB Webcam" });
  assert.equal(picks.value, "cam-usb");
  assert.deepEqual(picks.options.at(-1), { value: "cam-usb", label: `USB Webcam ${NOT_CONNECTED}` });
  assert.equal(cameraPicks([laptop], { id: "cam-usb", label: "" }).options.at(-1).label, `Your camera ${NOT_CONNECTED}`);
  assert.equal(cameraPicks([laptop, webcam], { id: "cam-usb", label: "USB Webcam" }).value, "cam-usb");
  assert.deepEqual(cameraPicks([{ deviceId: "x", label: "" }], null).options, [{ value: "", label: "Default" }, { value: "x", label: "Camera 1" }]);
});

test("choosing a camera remembers its name, and Default forgets the choice", () => {
  assert.deepEqual(cameraChoiceFor([laptop, webcam], "cam-usb", null), { id: "cam-usb", label: "USB Webcam" });
  assert.equal(cameraChoiceFor([laptop, webcam], "", { id: "cam-usb", label: "USB Webcam" }), null);
  assert.deepEqual(cameraChoiceFor([laptop], "cam-usb", { id: "cam-usb", label: "USB Webcam" }), { id: "cam-usb", label: "USB Webcam" });
});

test("the camera falls back while yours is gone and switches back when it returns", () => {
  const choice = { id: "cam-usb", label: "USB Webcam" };
  assert.deepEqual(cameraMove([laptop], choice, null), { deviceId: "" });
  assert.equal(cameraMove([laptop], choice, "cam-built-in"), null);
  assert.deepEqual(cameraMove([laptop, webcam], choice, "cam-built-in"), { deviceId: "cam-usb" });
  assert.equal(cameraMove([laptop, webcam], choice, "cam-usb"), null);
  assert.equal(cameraMove([], choice, null), null);
  assert.equal(cameraMove([laptop], null, "cam-built-in"), null);
  assert.deepEqual(cameraMove([laptop], null, "cam-usb"), { deviceId: "" });
  assert.equal(cameraMove([{ deviceId: "", label: "" }], choice, null), null);
});

test("the device choice script loads before the app and the pickers use it", () => {
  const html = readFileSync(join(ui, "index.html"), "utf8");
  const order = ["scripts/device-choice.js", "scripts/app.js"].map(src => html.indexOf(`<script src="${src}">`));
  assert.ok(order[0] > 0 && order[0] < order[1]);
  const app = readFileSync(join(ui, "scripts", "app.js"), "utf8");
  assert.ok(app.includes(`addEventListener("devicechange"`));
  assert.ok(!app.includes("names.includes(current"));
});
