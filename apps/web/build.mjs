import { execFileSync } from "node:child_process";
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { injectWebShell, webRelayList } from "./shell.mjs";

const web = import.meta.dirname;
const repo = join(web, "..", "..");
const ui = join(repo, "apps", "desktop", "ui");
const dist = process.env.COMMY_WEB_OUT || join(web, "dist");
const release = !process.argv.includes("--dev");
const relays = webRelayList(process.env.COMMY_WEB_RELAYS, process.env.COMMY_DEFAULT_RELAY || "https://relay.commy.chat:8443");
const base = process.env.COMMY_WEB_BASE || "";

const SKIPPED = ["apps/desktop/ui/tests/", "apps/desktop/ui/vendor/", "apps/desktop/ui/overlay.html"];

const run = (cmd, args, env = {}) => execFileSync(cmd, args, { cwd: repo, stdio: "inherit", env: { ...process.env, ...env } });

run("cargo", ["build", "-p", "commy-web-client", "--target", "wasm32-unknown-unknown", ...(release ? ["--release"] : [])], {
  COMMY_DEFAULT_RELAY: relays[0],
});
const profile = release ? "release" : "debug";
const wasm = join(repo, "target", "wasm32-unknown-unknown", profile, "commy_web_client.wasm");

rmSync(dist, { recursive: true, force: true });
const tracked = execFileSync("git", ["ls-files", "-z", "--", "apps/desktop/ui"], { cwd: repo })
  .toString()
  .split("\0")
  .filter(path => path && !SKIPPED.some(prefix => path.startsWith(prefix)));
for (const path of tracked) {
  const target = join(dist, path.slice("apps/desktop/ui/".length));
  mkdirSync(dirname(target), { recursive: true });
  cpSync(join(repo, path), target);
}
for (const [from, to] of [
  ["web-commands.js", "scripts/web-commands.js"],
  ["web-voice.js", "scripts/web-voice.js"],
  ["web-voice-worklet.js", "scripts/web-voice-worklet.js"],
  ["web-bridge.js", "scripts/web-bridge.js"],
  ["web.css", "styles/web.css"],
]) {
  cpSync(join(web, "ui", from), join(dist, to));
}
run("wasm-bindgen", ["--target", "web", "--no-typescript", "--out-dir", join(dist, "scripts", "pkg"), wasm]);

const index = join(dist, "index.html");
writeFileSync(index, injectWebShell(readFileSync(index, "utf8"), { relays, base }));
console.log(`web client built into ${dist}`);
