import assert from "node:assert/strict";
import { test } from "node:test";

import { parseDevices } from "./adb.mjs";
import { benchEnv } from "./bench.mjs";
import { DEFAULTS, parseArgs, resolveConfig, summarize } from "./config.mjs";
import { countActiveNotifications, crashes, parseEpochLog, registeredTags, since, wakes } from "./logs.mjs";

const LOG = [
  "--------- beginning of main",
  "  1761160000.100  4993  4993 I FirebaseInitProvider: FirebaseApp initialization successful",
  "  1761160001.250  4993  5000 I RustStdoutStderr: \x1b[2m2026-10-22T19:22:26Z\x1b[0m \x1b[32m INFO\x1b[0m commy_desktop: webview connected over the tauri bridge",
  "  1761160005.000  4993  5000 I RustStdoutStderr: INFO commy_desktop::push_wake: registered 2 wake-up tags",
  "  1761160009.000  1418  1500 I ActivityManager: Start proc 7241:chat.commy.mobile/u0a302 for broadcast {chat.commy.mobile/com.google.firebase.iid.FirebaseInstanceIdReceiver}",
  "  1761160009.430  7241  7241 I CommyWake: wake-up push received, foreground=false, notifications allowed=true",
  "  1761160020.000  9999  9999 E AndroidRuntime: FATAL EXCEPTION: main",
].join("\n");

test("logcat lines in epoch format become timed entries with colours removed", () => {
  const entries = parseEpochLog(LOG);
  assert.equal(entries.length, 6);
  assert.equal(entries[0].at, 1761160000100);
  assert.equal(entries[1].pid, 4993);
  assert.ok(!entries[1].text.includes("\x1b"));
});

test("the newest wake registration count is read", () => {
  assert.equal(registeredTags(parseEpochLog(LOG)), 2);
  assert.equal(registeredTags([]), null);
});

test("wake-ups carry their time and whether the app was in front", () => {
  const [w] = wakes(parseEpochLog(LOG));
  assert.deepEqual(w, { at: 1761160009430, pid: 7241, foreground: false, notificationsAllowed: true });
  assert.equal(wakes(since(parseEpochLog(LOG), 1761160010000)).length, 0);
});

test("only crashes from Commy's processes count", () => {
  const entries = parseEpochLog(LOG);
  assert.equal(crashes(entries, [4993]).length, 0, "another app's crash is ignored");
  assert.equal(crashes(entries, [9999]).length, 1);
});

test("active notifications skip the launcher's group summary", () => {
  const dump = [
    "  NotificationList:",
    "    NotificationRecord(0x1: pkg=chat.commy.mobile user=UserHandle{0} id=0 tag=g flags=LOCAL_ONLY|GROUP_SUMMARY|AUTOGROUP_SUMMARY)",
    "      icon=x",
    "    NotificationRecord(0x2: pkg=chat.commy.mobile user=UserHandle{0} id=7303 tag=null)",
    "    NotificationRecord(0x3: pkg=com.other.app user=UserHandle{0} id=1 tag=null)",
  ].join("\n");
  assert.equal(countActiveNotifications(dump, "chat.commy.mobile"), 1);
  assert.equal(countActiveNotifications("", "chat.commy.mobile"), 0);
});

test("adb devices output lists serials with their state", () => {
  const out = "List of devices attached\nTAB1234567\tdevice\nemulator-5554\toffline\n\n";
  assert.deepEqual(parseDevices(out), [
    { serial: "TAB1234567", state: "device" },
    { serial: "emulator-5554", state: "offline" },
  ]);
});

test("options pick an APK, scenarios and a report path", () => {
  assert.deepEqual(parseArgs(["--run", "123", "--only", "wake, burst", "--report", "r.json"]), { only: ["wake", "burst"], apk: null, run: "123", config: null, report: "r.json" });
  assert.throws(() => parseArgs(["--apk", "a.apk", "--run", "1"]), /not both/);
  assert.throws(() => parseArgs(["--only"]), /needs a value/);
  assert.throws(() => parseArgs(["--fast"]), /unknown option/);
});

test("scenarios that talk to a second person need a bench", () => {
  const known = DEFAULTS.scenarios;
  assert.equal(resolveConfig({}, { only: ["launch"] }, known).needsBench, false);
  assert.throws(() => resolveConfig({}, { only: ["wake"] }, known), /bench\.exe/);
  const config = resolveConfig({ bench: { exe: "b.exe", dataDir: "d" } }, { only: null }, known);
  assert.deepEqual(config.scenarios, known);
  assert.equal(config.bench.debugPort, 9334);
  assert.throws(() => resolveConfig({}, { only: ["nope"] }, known), /unknown scenario/);
});

test("the bench starts with its own data folder, server and debug port", () => {
  const env = benchEnv({ dataDir: "D", authUrl: "https://dev.example", debugPort: 9555 }, { PATH: "p" });
  assert.equal(env.PATH, "p");
  assert.equal(env.COMMY_DATA_DIR, "D");
  assert.equal(env.COMMY_AUTH_URL, "https://dev.example");
  assert.equal(env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS, "--remote-debugging-port=9555");
  assert.equal(benchEnv({ dataDir: "D", debugPort: 1 }, {}).COMMY_AUTH_URL, undefined);
});

test("the summary marks each scenario and counts failures", () => {
  const { text, failed } = summarize([
    { name: "launch", ok: true, ms: 12000 },
    { name: "wake", ok: false, ms: 3000, details: { reason: "no wake-up push within 180s" } },
  ]);
  assert.equal(failed, 1);
  assert.match(text, /PASS {2}launch/);
  assert.match(text, /FAIL {2}wake .* no wake-up push within 180s/);
  assert.match(text, /1 of 2 failed/);
});
