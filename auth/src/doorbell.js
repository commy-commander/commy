// Message doorbell: content-free wake-ups for offline messages. A device that
// sent messages while their recipient was offline rings the recipient's daily
// token with blinded message ids; the recipient's device takes them, then
// fetches the messages from its peers. Tokens and ids are keyed hashes made on
// the devices, so this server never learns which Commy server, account or
// message they belong to. Every request carries one token, needs no session,
// and nothing about the caller is stored.
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
// Waiting takes per token; a newer one replaces the oldest
export const MAX_WAITERS_PER_TOKEN = 4;
const PATHS = ["/commy/doorbell/ring", "/commy/doorbell/take"];

// Deletes rows older than MAX_AGE_MS.
export async function sweepDoorbell(adapter, now = Date.now()) {
  await adapter.deleteMany({
    model: MODEL,
    where: [{ field: "createdAt", operator: "lt", value: new Date(now - MAX_AGE_MS) }],
  });
}

// Reads a request body up to `max` bytes; null when it is larger.
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

// Waiting takes per token. A waiter is armed before the table is read, so a
// ring that lands during the read still wakes it.
function createBell(maxWaiters) {
  const waiting = new Map();
  let total = 0;
  return {
    arm(token) {
      let set = waiting.get(token);
      // A device that reconnects replaces its own stale wait
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
  sweepEveryMs = 60 * 60 * 1000,
  firstSweepMs = 5 * 60 * 1000,
  maxWaiters = 10_000,
  // Rings stored across all tokens; past it rings are refused until takes or
  // the sweep make room, so arbitrary tokens cannot fill the disk
  maxRows = 200_000,
} = {}) => {
  const bell = createBell(maxWaiters);
  // One read or write per token at a time, so two takes never return the same ids
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

  async function store(adapter, token, ids) {
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

  // Returns and deletes what is stored for a token. Rows are deleted by id, so
  // a ring that lands meanwhile is kept for the next take.
  async function takeStored(adapter, token) {
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
  }

  async function take(adapter, token, waitMs, signal) {
    const deadline = Date.now() + waitMs;
    let hello = false;
    for (;;) {
      const remaining = hello ? 0 : deadline - Date.now();
      const waiter = remaining > 0 ? bell.arm(token) : null;
      let ids;
      try {
        ids = await locked(token, () => takeStored(adapter, token));
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
      const sweep = () =>
        sweepDoorbell(context.adapter).catch((err) => console.warn(`[auth] doorbell sweep failed: ${err.message}`));
      setTimeout(sweep, firstSweepMs).unref();
      setInterval(sweep, sweepEveryMs).unref();
    },
    // Bodies are tiny, so anything larger is refused before it is parsed
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
          // An empty ring is a hello: it wakes a listening device and is never stored
          if (ids.length > 0) {
            const total = await ctx.context.adapter.count({ model: MODEL });
            if (total >= maxRows) throw new APIError("SERVICE_UNAVAILABLE", { message: "The doorbell is full, try again later" });
            await locked(token, () => store(ctx.context.adapter, token, ids));
          }
          bell.ring(token, ids.length === 0);
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
