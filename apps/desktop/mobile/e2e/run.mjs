import { execFile } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";

import { adb } from "./adb.mjs";
import { startBench } from "./bench.mjs";
import { parseArgs, resolveConfig, summarize } from "./config.mjs";
import { SCENARIOS } from "./scenarios.mjs";

const exec = promisify(execFile);
const ANDROID_ARTIFACT = "commy-android-debug";

async function apkFromRun(runId, repo) {
  const dir = mkdtempSync(join(tmpdir(), "commy-e2e-apk-"));
  await exec("gh", ["run", "download", runId, ...(repo ? ["-R", repo] : []), "-n", ANDROID_ARTIFACT, "-D", dir], { maxBuffer: 16 * 1024 * 1024 });
  const apk = readdirSync(dir).find(name => name.endsWith(".apk"));
  if (!apk) throw new Error(`run ${runId} has no APK in ${ANDROID_ARTIFACT}`);
  return join(dir, apk);
}

async function pickDevice(config) {
  const device = adb({ path: config.adb, serial: config.serial });
  const ready = (await device.devices()).filter(d => d.state === "device");
  if (config.serial && !ready.some(d => d.serial === config.serial)) throw new Error(`device ${config.serial} is not connected or not authorised`);
  if (!config.serial && ready.length !== 1) throw new Error(`expected one authorised device, found ${ready.length}; set "serial" in the config`);
  return config.serial ? device : adb({ path: config.adb, serial: ready[0].serial });
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const file = args.config ? JSON.parse(readFileSync(args.config, "utf8").replace(/^﻿/, "")) : {};
  const config = resolveConfig(file, args, Object.keys(SCENARIOS));
  const apk = args.apk ?? (args.run ? await apkFromRun(args.run, config.repo) : null);
  const device = await pickDevice(config);
  const bench = config.needsBench ? await startBench(config.bench) : null;
  const ctx = {
    device,
    apk,
    pkg: config.package,
    tabletDebugPort: config.tabletDebugPort,
    pushGapMs: config.pushGapSeconds * 1000,
    bench: bench?.app ?? null,
    sent: [],
    tabletUser: null,
    benchUser: null,
  };
  const results = [];
  await device.keepAwake(true);
  try {
    for (const name of config.scenarios) {
      const started = Date.now();
      process.stdout.write(`running ${name}...\n`);
      try {
        const { ok, details } = await SCENARIOS[name](ctx);
        results.push({ name, ok, details, ms: Date.now() - started });
      } catch (e) {
        results.push({ name, ok: false, error: e.message, ms: Date.now() - started });
      }
    }
  } finally {
    await device.keepAwake(false).catch(() => {});
    await bench?.stop();
  }
  const { text, failed } = summarize(results);
  process.stdout.write(`${text}\n`);
  const report = { at: new Date().toISOString(), apk, server: results.find(r => r.name === "accounts")?.details?.server ?? null, benchLog: bench?.log ?? null, results };
  if (args.report) writeFileSync(args.report, JSON.stringify(report, null, 2));
  process.exitCode = failed ? 1 : 0;
}

main().catch(e => {
  process.stderr.write(`${e.message}\n`);
  process.exitCode = 2;
});
