import http from "node:http";
import { getMigrations } from "better-auth/db/migration";
import { toNodeHandler } from "better-auth/node";
import { createAuth } from "./auth.js";
import { discoveryDocument, featureSummary, loadConfig } from "./config.js";
import { dropDeviceNames } from "./devices.js";

const config = loadConfig();
const { auth, options, pool } = createAuth(config);

async function migrate(attempts = 30) {
  for (let i = 1; ; i++) {
    try {
      const { toBeCreated, toBeAdded, runMigrations } = await getMigrations(options);
      if (toBeCreated.length || toBeAdded.length) {
        console.log(`[auth] applying schema: ${toBeCreated.length} new tables, ${toBeAdded.length} altered`);
        await runMigrations();
      }
      await dropDeviceNames(pool);
      return;
    } catch (err) {
      if (i >= attempts) throw err;
      console.warn(`[auth] database not ready (${err.code ?? err.message}), retrying`);
      await new Promise((r) => setTimeout(r, 2000));
    }
  }
}

const authHandler = toNodeHandler(auth);
const discovery = JSON.stringify(discoveryDocument(config));

function sendJson(res, status, body) {
  res.writeHead(status, {
    "content-type": "application/json",
    "access-control-allow-origin": "*",
    "cache-control": "public, max-age=300",
  });
  res.end(body);
}

const server = http.createServer(async (req, res) => {
  const path = (req.url ?? "/").split("?")[0];

  if (path === "/healthz") {
    try {
      await pool.query("select 1");
      return sendJson(res, 200, '{"ok":true}');
    } catch {
      return sendJson(res, 503, '{"ok":false}');
    }
  }
  if (path === "/.well-known/commy-auth.json") {
    return sendJson(res, 200, discovery);
  }
  if (path.startsWith(config.basePath)) {
    return authHandler(req, res);
  }
  sendJson(res, 404, '{"error":"not_found"}');
});

await migrate();
server.listen(config.port, "0.0.0.0", () => {
  console.log(`[auth] listening on :${config.port}, public URL ${config.baseURL}`);
  console.log(`[auth] ${featureSummary(config)}`);
  for (const warning of config.warnings) console.warn(`[auth] ${warning}`);
});

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    server.close();
    pool.end().finally(() => process.exit(0));
  });
}
