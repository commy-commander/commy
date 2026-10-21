import pg from "pg";
import { openEverySealedRow } from "./at-rest.js";
import { loadConfig } from "./config.js";

const config = loadConfig();
const pool = new pg.Pool({ connectionString: config.databaseUrl, max: 1 });
try {
  const opened = await openEverySealedRow(pool, config.secret);
  console.log(`[auth] decrypted ${opened} stored values; start an older auth server or change BETTER_AUTH_SECRET now`);
} finally {
  await pool.end();
}
