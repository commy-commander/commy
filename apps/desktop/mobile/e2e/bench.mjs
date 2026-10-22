import { spawn } from "node:child_process";
import { mkdirSync, openSync } from "node:fs";
import { join } from "node:path";

import { sleep } from "./adb.mjs";
import { connect, findAppPage } from "./cdp.mjs";

const READY_TIMEOUT_MS = 60_000;

export function benchEnv(config, base = process.env) {
  return {
    ...base,
    COMMY_DATA_DIR: config.dataDir,
    COMMY_INSTANCE_LABEL: config.label ?? "e2e bench",
    ...(config.authUrl ? { COMMY_AUTH_URL: config.authUrl } : {}),
    RUST_LOG: config.rustLog ?? "info,commy_desktop=debug,iroh=warn",
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${config.debugPort}`,
  };
}

async function waitForPage(port) {
  const deadline = Date.now() + READY_TIMEOUT_MS;
  for (;;) {
    try {
      await findAppPage(port);
      return;
    } catch (e) {
      if (Date.now() > deadline) throw new Error(`the bench app did not open its debug port ${port}: ${e.message}`);
      await sleep(1000);
    }
  }
}

export async function startBench(config) {
  if (config.attach) {
    await waitForPage(config.debugPort);
    return { app: await connect(config.debugPort), log: config.log ?? null, stop: async () => {} };
  }
  mkdirSync(config.dataDir, { recursive: true });
  const log = join(config.dataDir, "e2e-log.txt");
  const out = openSync(log, "w");
  const child = spawn(config.exe, [], { env: benchEnv(config), stdio: ["ignore", out, out], windowsHide: false });
  await waitForPage(config.debugPort);
  const app = await connect(config.debugPort);
  return {
    app,
    log,
    async stop() {
      app.close();
      child.kill();
    },
  };
}
