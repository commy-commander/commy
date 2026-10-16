// Optional Redis for the short-lived, hot-path state of the account server:
// doorbell rings and rate-limit counters. Everything kept here expires on its
// own and holds nothing an operator could read back to a person: doorbell
// tokens and ids are random values made on the devices, and rate-limit keys
// are keyed hashes. Durable account data stays in Postgres. Without REDIS_URL
// the server keeps both in the places it used them before (the database and
// process memory), so Redis is an upgrade, not a requirement.
import { createClient } from "redis";

// Opens one client for commands and one for subscriptions (a subscribed
// connection cannot run commands). Both reconnect on their own.
export async function connectRedis(url, { log = console } = {}) {
  const client = createClient({ url, socket: { reconnectStrategy: (tries) => Math.min(500 * 2 ** tries, 10_000) } });
  const subscriber = client.duplicate();
  let warned = 0;
  for (const c of [client, subscriber]) {
    c.on("error", (err) => {
      // One line per minute, not one per failed command
      if (Date.now() - warned < 60_000) return;
      warned = Date.now();
      log.warn(`[auth] redis: ${err.message}`);
    });
  }
  await Promise.all([client.connect(), subscriber.connect()]);
  return {
    client,
    subscriber,
    async ping() {
      return (await client.ping()) === "PONG";
    },
    async close() {
      await Promise.allSettled([client.quit(), subscriber.quit()]);
    },
  };
}
