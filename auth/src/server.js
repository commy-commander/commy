import http from "node:http";
import { getMigrations } from "better-auth/db/migration";
import { toNodeHandler } from "better-auth/node";
import pg from "pg";
import { sealExistingRows } from "./at-rest.js";
import { createAuth } from "./auth.js";
import { discoveryDocument, featureSummary, loadConfig } from "./config.js";
import { dropDeviceNames } from "./devices.js";
import { createRequestHandler } from "./http.js";
import { createLinkBlocklist } from "./link-blocklist.js";
import { connectRedis } from "./redis.js";

const config = loadConfig();
const SEAL_ONCE_OLDER_CONTAINERS_HAVE_STOPPED_MS = 10 * 60 * 1000;

async function retryUntilDatabaseReady(work, attempts) {
  for (let i = 1; ; i++) {
    try {
      await work();
      return;
    } catch (err) {
      if (i >= attempts) throw err;
      console.warn(`[auth] database not ready (${err.code ?? err.message}), retrying`);
      await new Promise((r) => setTimeout(r, 2000));
    }
  }
}

async function dropRetiredColumnsBeforeBetterAuthStarts(attempts = 30) {
  const old = new pg.Pool({ connectionString: config.databaseUrl, max: 1 });
  try {
    await retryUntilDatabaseReady(() => dropDeviceNames(old), attempts);
  } finally {
    await old.end();
  }
}

await dropRetiredColumnsBeforeBetterAuthStarts();
const redis = config.redisUrl ? connectRedis(config.redisUrl) : null;
redis?.ready.then(() => console.log("[auth] redis connected"));
const { auth, options, pool } = createAuth(config, { redis });

function sealRowsFromOlderServers() {
  sealExistingRows(pool, config.secret)
    .then((sealed) => sealed && console.log(`[auth] encrypted ${sealed} stored values written by an older auth server`))
    .catch((err) => console.warn(`[auth] encrypting older rows failed, retrying at the next start: ${err.message}`));
}

async function migrate(attempts = 30) {
  await retryUntilDatabaseReady(async () => {
    const { toBeCreated, toBeAdded, runMigrations } = await getMigrations({ ...options, database: pool });
    if (toBeCreated.length || toBeAdded.length) {
      console.log(`[auth] applying schema: ${toBeCreated.length} new tables, ${toBeAdded.length} altered`);
      await runMigrations();
    }
  }, attempts);
}

const linkBlocklist = createLinkBlocklist({ sources: config.linkBlocklistSources });

const server = http.createServer(
  createRequestHandler({
    authHandler: toNodeHandler(auth),
    basePath: config.basePath,
    discovery: JSON.stringify(discoveryDocument(config)),
    pool,
    redis,
    linkBlocklist,
  }),
);

await migrate();
server.listen(config.port, "0.0.0.0", () => {
  console.log(`[auth] listening on :${config.port}, public URL ${config.baseURL}`);
  console.log(`[auth] ${featureSummary(config)}`);
  for (const warning of config.warnings) console.warn(`[auth] ${warning}`);
  setTimeout(sealRowsFromOlderServers, SEAL_ONCE_OLDER_CONTAINERS_HAVE_STOPPED_MS).unref();
  linkBlocklist.start();
});

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    server.close();
    linkBlocklist.stop();
    Promise.allSettled([pool.end(), redis?.close()]).then(() => process.exit(0));
  });
}
