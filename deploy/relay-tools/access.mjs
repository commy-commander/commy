import fs from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import { Answers, combineAnswers, emptyTally, ENDPOINT_ID, letIn, loadAccessConfig, tally, tallyReport } from "./lib.mjs";

const cfg = loadAccessConfig(process.env);
const answers = new Answers();
const WRITE_EVERY_MS = 60 * 1000;
let today = emptyTally(new Date());

async function askOne(url, endpoint) {
  try {
    const res = await fetch(url, {
      method: "POST",
      headers: { authorization: `Bearer ${cfg.token}`, "x-iroh-endpoint-id": endpoint },
      signal: AbortSignal.timeout(cfg.timeoutMs),
    });
    if (!res.ok) throw new Error(`auth answered ${res.status}`);
    const { linked } = await res.json();
    return typeof linked === "boolean" ? linked : undefined;
  } catch (err) {
    console.warn(`[relay-access] ${new URL(url).host} could not check an endpoint: ${err.message}`);
    return undefined;
  }
}

async function askAuth(endpoint) {
  return combineAnswers(await Promise.all(cfg.authUrls.map((url) => askOne(url, endpoint))));
}

async function linkedState(endpoint) {
  const now = Date.now();
  const kept = answers.get(endpoint, now);
  if (kept !== undefined) return kept;
  const linked = await askAuth(endpoint);
  if (linked !== undefined) answers.set(endpoint, linked, now);
  return linked;
}

async function decide(endpoint) {
  const linked = await linkedState(endpoint);
  const allowed = letIn(cfg.mode, linked);
  today = tally(today, new Date(), endpoint, { linked, allowed });
  return allowed;
}

async function writeReport() {
  await fs.mkdir(path.dirname(cfg.stateFile), { recursive: true });
  const tmp = `${cfg.stateFile}.tmp`;
  await fs.writeFile(tmp, JSON.stringify(tallyReport(today, cfg.mode), null, 2));
  await fs.rename(tmp, cfg.stateFile);
}

const server = http.createServer((req, res) => {
  if (req.method === "GET" && req.url === "/healthz") {
    res.writeHead(200).end("ok");
    return;
  }
  const endpoint = String(req.headers["x-iroh-endpoint-id"] ?? "").trim().toLowerCase();
  if (req.method !== "POST" || !ENDPOINT_ID.test(endpoint)) {
    res.writeHead(400).end("false");
    return;
  }
  decide(endpoint).then(
    (allowed) => res.writeHead(allowed ? 200 : 403, { "content-type": "text/plain" }).end(allowed ? "true" : "false"),
    () => res.writeHead(200, { "content-type": "text/plain" }).end("true"),
  );
});

if (!cfg.token) console.warn("[relay-access] RELAY_ACCESS_TOKEN is not set, so every check fails and lets the endpoint in");
setInterval(() => writeReport().catch((err) => console.warn(`[relay-access] ${err.message}`)), WRITE_EVERY_MS).unref();
server.listen(cfg.port, () => console.log(`[relay-access] :${cfg.port} ${cfg.mode} mode, asking ${cfg.authUrls.map((url) => new URL(url).host).join(", ")}`));
