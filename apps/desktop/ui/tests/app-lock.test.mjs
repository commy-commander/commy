import "./i18n-global.mjs";
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  APP_LOCK_HINT_KEY, APP_LOCK_AFTER_CHOICES, appLockButtonText, appLockSettingText,
  appLockHint, appLockCoversOnLeave, appLockShouldAsk, appLockCanceled,
} = require("../scripts/app-lock.js");

const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("index.html");
const code = read("scripts", "app-lock.js");
const rust = read("..", "src-tauri", "src", "app_lock.rs");
const lib = read("..", "src-tauri", "src", "lib.rs");
const plist = read("..", "src-tauri", "Info.ios.plist");
const swift = read("..", "app-lock", "ios", "Sources", "AppLockPlugin.swift");
const kotlin = read("..", "mobile", "android", "AppLockPlugin.kt");

const status = extra => ({ supported: true, phone_secure: true, unlock_with: "face", enabled: true, after_secs: 0, locked: false, asking: false, ...extra });

test("the unlock button names the way this phone unlocks", () => {
  assert.equal(appLockButtonText("face"), "Unlock with Face ID");
  assert.equal(appLockButtonText("touch"), "Unlock with Touch ID");
  assert.equal(appLockButtonText("fingerprint"), "Unlock with fingerprint");
  assert.equal(appLockButtonText("passcode"), "Unlock with passcode");
  assert.equal(appLockButtonText("biometrics"), "Unlock");
  assert.equal(appLockButtonText("something new"), "Unlock with passcode");
});

test("the setting says how it unlocks, that the passcode works too, and about the app switcher", () => {
  const face = appLockSettingText(status());
  assert.match(face, /Face ID/);
  assert.match(face, /passcode works too/);
  assert.match(face, /app switcher/);
  assert.doesNotMatch(appLockSettingText(status({ unlock_with: "passcode" })), /works too/);
  assert.match(appLockSettingText(status({ phone_secure: false })), /^Set a passcode on your phone first/);
  assert.match(appLockSettingText(null), /^Set a passcode/);
});

test("the quick start hint is kept only while the lock is on in the phone apps", () => {
  assert.equal(APP_LOCK_HINT_KEY, "commy.appLockOn");
  assert.equal(appLockHint(status()), "1");
  assert.equal(appLockHint(status({ enabled: false })), "");
  assert.equal(appLockHint(status({ supported: false })), "");
  assert.equal(appLockHint(null), "");
  assert.equal(appLockCoversOnLeave(status()), true);
  assert.equal(appLockCoversOnLeave(status({ enabled: false })), false);
});

test("the unlock prompt opens by itself once per lock and never while one is open", () => {
  const locked = status({ locked: true });
  assert.equal(appLockShouldAsk({ status: locked, askOnLock: true, asking: false }), true);
  assert.equal(appLockShouldAsk({ status: locked, askOnLock: false, asking: false }), false);
  assert.equal(appLockShouldAsk({ status: locked, askOnLock: true, asking: true }), false);
  assert.equal(appLockShouldAsk({ status: status({ locked: true, asking: true }), askOnLock: true, asking: false }), false);
  assert.equal(appLockShouldAsk({ status: status(), askOnLock: true, asking: false }), false);
  assert.equal(appLockShouldAsk({ status: null, askOnLock: true, asking: false }), false);
});

test("a closed prompt is not shown as an error", () => {
  assert.equal(appLockCanceled("canceled"), true);
  assert.equal(appLockCanceled("Set a passcode on your phone first, then turn this on."), false);
});

test("the lock time choices match between the page, the script and the app", () => {
  const options = [...html.matchAll(/<select id="app-lock-after"[\s\S]*?<\/select>/g)][0][0];
  const values = [...options.matchAll(/<option value="(\d+)"[^>]*>([^<]+)<\/option>/g)].map(m => ({ secs: Number(m[1]), label: m[2] }));
  assert.deepEqual(values, APP_LOCK_AFTER_CHOICES);
  const rustChoices = rust.match(/LOCK_AFTER_CHOICES: \[u64; \d+\] = \[([^\]]+)\]/)[1].split(",").map(n => Number(n.trim()));
  assert.deepEqual(rustChoices, APP_LOCK_AFTER_CHOICES.map(c => c.secs));
});

test("the setting is only in the phone apps and starts hidden until the app says it can lock", () => {
  const card = html.match(/<div class="([^"]+)" id="app-lock-card">/)[1].split(" ");
  for (const name of ["mobile-only", "native-only", "app-lock-part", "hidden"]) assert.ok(card.includes(name), name);
  assert.ok(html.includes('id="app-lock-toggle"'));
  assert.match(html, /<div class="startup app-lock hidden" id="app-lock" role="dialog" aria-modal="true"/);
  assert.ok(html.indexOf("scripts/app-lock.js") < html.indexOf("scripts/app.js"));
});

test("the commands the page calls are the ones the app offers", () => {
  const called = [...new Set([...code.matchAll(/"(app_lock_[a-z_]+)"/g)].map(m => m[1]))].sort();
  assert.deepEqual(called, ["app_lock_away", "app_lock_back", "app_lock_set", "app_lock_status", "app_lock_unlock"]);
  for (const command of called) {
    assert.ok(rust.includes(`pub async fn ${command}(`) || rust.includes(`pub fn ${command}(`), command);
    assert.ok(lib.includes(`app_lock::${command},`), command);
  }
  assert.ok(rust.includes('const CHANGED_EVENT: &str = "app-lock-changed";'));
  assert.ok(code.includes('const APP_LOCK_CHANGED_EVENT = "app-lock-changed";'));
});

test("both phones accept their own passcode when Face ID or a fingerprint does not work", () => {
  assert.match(plist, /<key>NSFaceIDUsageDescription<\/key>\s*<string>[^<]+<\/string>/);
  assert.ok(swift.includes("evaluatePolicy(.deviceOwnerAuthentication,"));
  assert.ok(!swift.includes("evaluatePolicy(.deviceOwnerAuthenticationWithBiometrics"));
  assert.ok(kotlin.includes("BIOMETRIC_WEAK or BiometricManager.Authenticators.DEVICE_CREDENTIAL"));
});

test("both phones hide Commy in the app switcher while the lock is on", () => {
  assert.ok(swift.includes("UIApplication.willResignActiveNotification"));
  assert.ok(swift.includes("UIVisualEffectView"));
  assert.ok(kotlin.includes("setRecentsScreenshotEnabled(!on)"));
  assert.ok(kotlin.includes("FLAG_SECURE"));
});

test("the app lock code has no comments", () => {
  for (const source of [code, rust, swift, kotlin]) assert.doesNotMatch(source, /^\s*(\/\/|\/\*)/m);
});
