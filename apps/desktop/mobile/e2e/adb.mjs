import { execFile } from "node:child_process";
import { promisify } from "node:util";

const exec = promisify(execFile);
const SIGNATURE_CHANGED = /INSTALL_FAILED_UPDATE_INCOMPATIBLE|signatures do not match/;
const SETTLE_AFTER_HOME_MS = 1500;

export const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

export function parseDevices(text) {
  return text
    .split(/\r?\n/)
    .slice(1)
    .map(line => line.trim().split(/\s+/))
    .filter(parts => parts.length >= 2 && parts[0])
    .map(([serial, state]) => ({ serial, state }));
}

export function adb({ path = process.env.ADB || "adb", serial } = {}) {
  const target = serial ? ["-s", serial] : [];
  const call = async (args, { binary = false, timeout = 180_000 } = {}) => {
    const { stdout } = await exec(path, [...target, ...args], { encoding: binary ? "buffer" : "utf8", maxBuffer: 256 * 1024 * 1024, timeout });
    return stdout;
  };
  const shell = command => call(["shell", command]);

  return {
    call,
    shell,
    async devices() {
      const { stdout } = await exec(path, ["devices"], { encoding: "utf8" });
      return parseDevices(stdout);
    },
    async deviceNowMs() {
      return Number((await shell("date +%s%3N")).trim());
    },
    async pid(pkg) {
      const out = (await shell(`pidof ${pkg}`).catch(() => "")).trim();
      return out ? Number(out.split(/\s+/)[0]) : null;
    },
    async install(apk, pkg) {
      try {
        await call(["install", "-r", apk]);
        return "updated";
      } catch (e) {
        if (!SIGNATURE_CHANGED.test(`${e.stderr ?? ""}${e.stdout ?? ""}${e.message}`)) throw e;
        await call(["uninstall", pkg]);
        await call(["install", apk]);
        return "reinstalled";
      }
    },
    async keepAwake(on) {
      await shell(`svc power stayon ${on ? "usb" : "false"}`);
    },
    async wakeScreen() {
      await shell("input keyevent KEYCODE_WAKEUP");
      await shell("wm dismiss-keyguard").catch(() => {});
    },
    async launch(pkg) {
      await this.wakeScreen();
      return shell(`monkey -p ${pkg} -c android.intent.category.LAUNCHER 1`);
    },
    async closeLikeSwipe(pkg) {
      await shell("input keyevent KEYCODE_HOME");
      await sleep(SETTLE_AFTER_HOME_MS);
      await shell(`am kill ${pkg}`);
      await sleep(SETTLE_AFTER_HOME_MS);
      const pid = (await shell(`pidof ${pkg}`).catch(() => "")).trim();
      if (pid) await shell(`run-as ${pkg} kill -9 ${pid.split(/\s+/).join(" ")}`).catch(() => {});
    },
    clearLog() {
      return call(["logcat", "-c"]);
    },
    readLog() {
      return call(["logcat", "-d", "-v", "epoch"]);
    },
    async forwardWebview(pid, port) {
      await call(["forward", `tcp:${port}`, `localabstract:webview_devtools_remote_${pid}`]);
      return async () => {
        await call(["forward", "--remove", `tcp:${port}`]).catch(() => {});
      };
    },
    notifications() {
      return shell("dumpsys notification --noredact");
    },
    screenshot() {
      return call(["exec-out", "screencap", "-p"], { binary: true });
    },
  };
}
