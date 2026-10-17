import * as z from "zod";
import { APIError, createAuthEndpoint } from "better-auth/api";

const MODEL = "doorbellRing";
const DAY_MS = 24 * 60 * 60 * 1000;
const TOKEN = /^[0-9a-f]{32}$/;
const BLINDED_ID = /^[0-9a-f]{24}$/;
export const MAX_IDS = 50;
const MAX_WAIT_SECONDS = 25;
export const MAX_PER_TOKEN = 1000;
export const MAX_AGE_MS = 14 * DAY_MS;
export const MAX_BODY_BYTES = 8 * 1024;
export const MAX_WAITERS_PER_TOKEN = 4;
const PATHS = ["/commy/doorbell/ring", "/commy/doorbell/take"];

export async function sweepDoorbell(adapter, now = Date.now()) {
  await adapter.deleteMany({
    model: MODEL,
    where: [{ field: "createdAt", operator: "lt", value: new Date(now - MAX_AGE_MS) }],
  });
}

export function databaseStore(adapter, { maxRows }) {
  async function store(token, ids) {
    const existing = await adapter.findMany({
      model: MODEL,
      where: [
        { field: "token", value: token },
        { field: "blindedId", operator: "in", value: ids },
      ],
      limit: ids.length,
    });
    const have = new Set(existing.map((row) => row.blindedId));
    const createdAt = new Date();
    await Promise.all(
      ids
        .filter((id) => !have.has(id))
        .map((blindedId) => adapter.create({ model: MODEL, data: { token, blindedId, createdAt } })),
    );
    const count = await adapter.count({ model: MODEL, where: [{ field: "token", value: token }] });
    if (count <= MAX_PER_TOKEN) return;
    const oldest = await adapter.findMany({
      model: MODEL,
      where: [{ field: "token", value: token }],
      sortBy: { field: "createdAt", direction: "asc" },
      limit: count - MAX_PER_TOKEN,
    });
    await adapter.deleteMany({ model: MODEL, where: [{ field: "id", operator: "in", value: oldest.map((row) => row.id) }] });
  }

  return {
    name: "database",
    async ring(token, ids) {
      const total = await adapter.count({ model: MODEL });
      if (total >= maxRows) return false;
      await store(token, ids);
      return true;
    },
    async take(token) {
      const rows = await adapter.findMany({
        model: MODEL,
        where: [{ field: "token", value: token }],
        sortBy: { field: "createdAt", direction: "asc" },
        limit: MAX_PER_TOKEN,
      });
      if (rows.length === 0) return [];
      await adapter.deleteMany({ model: MODEL, where: [{ field: "id", operator: "in", value: rows.map((row) => row.id) }] });
      const cutoff = Date.now() - MAX_AGE_MS;
      const fresh = rows.filter((row) => new Date(row.createdAt).getTime() >= cutoff);
      return [...new Set(fresh.map((row) => row.blindedId))];
    },
    sweep: (now) => sweepDoorbell(adapter, now),
  };
}

export const REDIS_PREFIX = "doorbell:";
const TOKENS_KEY = `${REDIS_PREFIX}tokens`;
const TOTAL_KEY = `${REDIS_PREFIX}total`;
export const RING_CHANNEL = `${REDIS_PREFIX}ring`;
const tokenKey = (token) => `${REDIS_PREFIX}t:${token}`;

const RING_SCRIPT = `
local now, cap, maxRows = tonumber(ARGV[1]), tonumber(ARGV[2]), tonumber(ARGV[3])
if tonumber(redis.call('GET', KEYS[3]) or '0') >= maxRows then return -1 end
local added = 0
for i = 5, #ARGV do added = added + redis.call('ZADD', KEYS[1], 'NX', now, ARGV[i]) end
local size = redis.call('ZCARD', KEYS[1])
if size > cap then
  redis.call('ZREMRANGEBYRANK', KEYS[1], 0, size - cap - 1)
  added = added - (size - cap)
end
if added ~= 0 then redis.call('INCRBY', KEYS[3], added) end
redis.call('ZADD', KEYS[2], now, ARGV[4])
return added
`;

const TAKE_SCRIPT = `
local stale = redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', '(' .. ARGV[1])
local ids = redis.call('ZRANGE', KEYS[1], 0, -1)
redis.call('DEL', KEYS[1])
redis.call('ZREM', KEYS[2], ARGV[2])
local gone = stale + #ids
if gone > 0 and redis.call('DECRBY', KEYS[3], gone) < 0 then redis.call('SET', KEYS[3], 0) end
return ids
`;

const SWEEP_SCRIPT = `
local gone = 0
for i = 3, #KEYS do
  local last = redis.call('ZSCORE', KEYS[1], ARGV[i - 1])
  if not last or tonumber(last) < tonumber(ARGV[1]) then
    gone = gone + redis.call('ZCARD', KEYS[i])
    redis.call('DEL', KEYS[i])
    redis.call('ZREM', KEYS[1], ARGV[i - 1])
  end
end
if gone > 0 and redis.call('DECRBY', KEYS[2], gone) < 0 then redis.call('SET', KEYS[2], 0) end
return gone
`;

export function redisStore(redis, { maxRows }) {
  const { client } = redis;
  return {
    name: "redis",
    async ring(token, ids) {
      const added = await client.eval(RING_SCRIPT, {
        keys: [tokenKey(token), TOKENS_KEY, TOTAL_KEY],
        arguments: [String(Date.now()), String(MAX_PER_TOKEN), String(maxRows), token, ...ids],
      });
      return Number(added) >= 0;
    },
    async take(token) {
      return client.eval(TAKE_SCRIPT, {
        keys: [tokenKey(token), TOKENS_KEY, TOTAL_KEY],
        arguments: [String(Date.now() - MAX_AGE_MS), token],
      });
    },
    async sweep(now = Date.now()) {
      const cutoff = String(now - MAX_AGE_MS);
      for (;;) {
        const tokens = await client.zRangeByScore(TOKENS_KEY, "-inf", `(${cutoff}`, { LIMIT: { offset: 0, count: 500 } });
        if (tokens.length === 0) return;
        await client.eval(SWEEP_SCRIPT, { keys: [TOKENS_KEY, TOTAL_KEY, ...tokens.map(tokenKey)], arguments: [cutoff, ...tokens] });
        if (tokens.length < 500) return;
      }
    },
    publish: (token, hello) => client.publish(RING_CHANNEL, `${hello ? "h" : "r"}${token}`),
    subscribe: (onRing) =>
      redis.ready.then(() =>
        redis.subscriber.subscribe(RING_CHANNEL, (message) => {
          if (TOKEN.test(message.slice(1))) onRing(message.slice(1), message[0] === "h");
        }),
      ),
  };
}

async function readCapped(request, max) {
  if (Number(request.headers.get("content-length") ?? 0) > max) return null;
  if (!request.body) return undefined;
  const reader = request.body.getReader();
  const chunks = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > max) {
      await reader.cancel().catch(() => {});
      return null;
    }
    chunks.push(value);
  }
  return Buffer.concat(chunks);
}

function createBell(maxWaiters) {
  const waiting = new Map();
  let total = 0;
  return {
    arm(token) {
      let set = waiting.get(token);
      if (set && set.size >= MAX_WAITERS_PER_TOKEN) set.values().next().value.settle("replaced");
      if (total >= maxWaiters) return null;
      set = waiting.get(token);
      if (!set) waiting.set(token, (set = new Set()));
      let resolve;
      const waiter = { promise: new Promise((r) => (resolve = r)) };
      waiter.settle = (reason) => {
        if (!set.delete(waiter)) return;
        if (set.size === 0 && waiting.get(token) === set) waiting.delete(token);
        total--;
        resolve(reason);
      };
      set.add(waiter);
      total++;
      return waiter;
    },
    ring(token, hello) {
      for (const waiter of [...(waiting.get(token) ?? [])]) waiter.settle(hello ? "hello" : "ring");
    },
  };
}

function waitFor(waiter, ms, signal) {
  const timer = setTimeout(() => waiter.settle("timeout"), ms);
  const abort = () => waiter.settle("abort");
  if (signal?.aborted) abort();
  signal?.addEventListener("abort", abort, { once: true });
  return waiter.promise.finally(() => {
    clearTimeout(timer);
    signal?.removeEventListener("abort", abort);
  });
}

export const commyDoorbell = ({
  redis = null,
  sweepEveryMs = 60 * 60 * 1000,
  firstSweepMs = 5 * 60 * 1000,
  maxWaiters = 10_000,
  maxRows = 200_000,
} = {}) => {
  const bell = createBell(maxWaiters);
  const locks = new Map();

  async function locked(token, task) {
    const run = (locks.get(token) ?? Promise.resolve()).then(task);
    const tail = run.catch(() => {});
    locks.set(token, tail);
    try {
      return await run;
    } finally {
      if (locks.get(token) === tail) locks.delete(token);
    }
  }

  let store = null;
  const storeFor = (adapter) => (store ??= redis ? redisStore(redis, { maxRows }) : databaseStore(adapter, { maxRows }));

  async function take(adapter, token, waitMs, signal) {
    const deadline = Date.now() + waitMs;
    let hello = false;
    for (;;) {
      const remaining = hello ? 0 : deadline - Date.now();
      const waiter = remaining > 0 ? bell.arm(token) : null;
      let ids;
      try {
        ids = await locked(token, () => storeFor(adapter).take(token));
      } catch (err) {
        waiter?.settle("error");
        throw err;
      }
      if (ids.length > 0 || remaining <= 0) {
        waiter?.settle("done");
        return { ids, hello };
      }
      if (!waiter) throw new APIError("SERVICE_UNAVAILABLE", { message: "The doorbell is busy, try again soon" });
      const reason = await waitFor(waiter, remaining, signal);
      if (reason === "hello") hello = true;
      else if (reason !== "ring") return { ids: [], hello: false };
    }
  }

  return {
    id: "commy-doorbell",
    schema: {
      [MODEL]: {
        fields: {
          token: { type: "string", required: true, index: true },
          blindedId: { type: "string", required: true },
          createdAt: { type: "date", required: true, index: true },
        },
      },
    },
    init(context) {
      const current = storeFor(context.adapter);
      const sweep = () => current.sweep().catch((err) => console.warn(`[auth] doorbell sweep failed: ${err.message}`));
      setTimeout(sweep, firstSweepMs).unref();
      setInterval(sweep, sweepEveryMs).unref();
      if (current.subscribe) {
        current.subscribe((token, hello) => bell.ring(token, hello)).catch((err) => console.warn(`[auth] doorbell wake-ups: ${err.message}`));
      }
    },
    async onRequest(request, context) {
      const basePath = context.options.basePath ?? "/api/auth";
      const path = new URL(request.url).pathname;
      if (!PATHS.some((p) => path === `${basePath}${p}`)) return;
      const body = await readCapped(request, MAX_BODY_BYTES);
      if (body === null) {
        return {
          response: Response.json({ message: "Request body is too large" }, { status: 413 }),
        };
      }
      return {
        request: new Request(request.url, {
          method: request.method,
          headers: request.headers,
          body,
          signal: request.signal,
        }),
      };
    },
    endpoints: {
      ringCommyDoorbell: createAuthEndpoint(
        "/commy/doorbell/ring",
        {
          method: "POST",
          body: z.object({
            token: z.string().regex(TOKEN),
            ids: z.array(z.string().regex(BLINDED_ID)).max(MAX_IDS),
          }),
        },
        async (ctx) => {
          const { token } = ctx.body;
          const ids = [...new Set(ctx.body.ids)];
          const current = storeFor(ctx.context.adapter);
          if (ids.length > 0) {
            const stored = await locked(token, () => current.ring(token, ids));
            if (!stored) throw new APIError("SERVICE_UNAVAILABLE", { message: "The doorbell is full, try again later" });
          }
          if (current.publish) await current.publish(token, ids.length === 0);
          else bell.ring(token, ids.length === 0);
          return ctx.json({ ok: true });
        },
      ),

      takeCommyDoorbell: createAuthEndpoint(
        "/commy/doorbell/take",
        {
          method: "POST",
          body: z.object({
            token: z.string().regex(TOKEN),
            wait: z.number().min(0).max(MAX_WAIT_SECONDS).optional(),
          }),
        },
        async (ctx) => {
          const waitMs = Math.round((ctx.body.wait ?? 0) * 1000);
          const result = await take(ctx.context.adapter, ctx.body.token, waitMs, ctx.request?.signal);
          return ctx.json(result);
        },
      ),
    },
  };
};
