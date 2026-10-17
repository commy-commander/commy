import { createClient } from "redis";

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
