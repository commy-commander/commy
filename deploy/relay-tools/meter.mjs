import fs from "node:fs/promises";
import path from "node:path";
import { describe, initialState, levelFor, loadConfig, parseMetrics, step } from "./lib.mjs";

const cfg = loadConfig(process.env);

async function readState() {
  try {
    return JSON.parse(await fs.readFile(cfg.stateFile, "utf8"));
  } catch {
    return initialState(new Date());
  }
}

async function writeState(state) {
  await fs.mkdir(path.dirname(cfg.stateFile), { recursive: true });
  const tmp = `${cfg.stateFile}.tmp`;
  await fs.writeFile(tmp, JSON.stringify(state, null, 2));
  await fs.rename(tmp, cfg.stateFile);
}

async function setPaused(paused) {
  if (paused) {
    await fs.mkdir(path.dirname(cfg.flagFile), { recursive: true });
    await fs.writeFile(cfg.flagFile, new Date().toISOString());
  } else {
    await fs.rm(cfg.flagFile, { force: true });
  }
}

function bodyMostWebhooksAccept(message) {
  return JSON.stringify({ content: message, text: message });
}

async function alert(message) {
  console.log(`[meter] ${message}`);
  if (!cfg.webhookUrl) return;
  try {
    await fetch(cfg.webhookUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: bodyMostWebhooksAccept(message),
      signal: AbortSignal.timeout(10000),
    });
  } catch (err) {
    console.warn(`[meter] alert failed: ${err.message}`);
  }
}

async function sample() {
  try {
    const res = await fetch(cfg.metricsUrl, { signal: AbortSignal.timeout(10000) });
    return res.ok ? parseMetrics(await res.text()) : null;
  } catch {
    return null;
  }
}

async function tick(state) {
  const now = new Date();
  const next = step(state, await sample(), now);
  next.level = levelFor(next.total, cfg.limitBytes, cfg.warnPercent, cfg.pausePercent);
  next.updatedAt = now.toISOString();
  const meterPausedItLastTick = state.level === "paused";
  if (next.level === "paused") await setPaused(true);
  else if (meterPausedItLastTick) await setPaused(false);
  if (next.level !== state.level) await alert(describe(next, cfg.limitBytes));
  await writeState(next);
  return next;
}

let state = await readState();
console.log(`[meter] watching ${cfg.metricsUrl}, limit ${cfg.limitBytes / 1e9} GB per month`);
for (;;) {
  try {
    state = await tick(state);
  } catch (err) {
    console.warn(`[meter] ${err.message}`);
  }
  await new Promise((r) => setTimeout(r, cfg.intervalMs));
}
