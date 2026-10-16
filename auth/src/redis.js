// Optional Redis for the short-lived, hot-path state of the account server:
// doorbell rings and rate-limit counters. Everything kept here expires on its
// own and holds nothing an operator could read back to a person: doorbell
// tokens and ids are random values made on the devices, and rate-limit keys
// are keyed hashes. Durable account data stays in Postgres. Without REDIS_URL
// the server keeps both in the places it used them before (the database and
// process memory), so Redis is an upgrade, not a requirement.
import { createClient } from "redis";

// Opens one client for commands and one for subscriptions (a subscribed
// connection cannot run commands). Both reconnect on their own. The handle is
// returned at once and `ready` resolves on the first connection, so the server
// can start serving sign-ins while Redis is still coming up; a command sent
// while disconnected fails instead of waiting.
export function connectRedis(url, { log = console } = {}) {
  const options = {
    url,
    disableOfflineQueue: true,
    socket: { reconnectStrategy: (tries) => Math.min(500 * 2 ** tries, 10_000) },
  };
  const client = createClient(options);
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
  const ready = Promise.all([client.connect(), subscriber.connect()]).then(() => {});
  ready.catch(() => {});
  return {
    client,
    subscriber,
    ready,
    async ping() {
      return (await client.ping()) === "PONG";
    },
    async close() {
      await Promise.allSettled([client.quit(), subscriber.quit()]);
    },
  };
}
