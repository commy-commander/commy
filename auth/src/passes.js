import { constants, createPrivateKey, createPublicKey, generateKeyPair, privateDecrypt } from "node:crypto";
import { promisify } from "node:util";
import * as z from "zod";
import { APIError, createAuthEndpoint, sessionMiddleware } from "better-auth/api";
import { isSuspended, suspendedRefusal } from "./suspension.js";

export const PASS_KEYS_PATH = "/commy/passes/keys";
export const PASS_ISSUE_PATH = "/commy/passes";
export const PASS_KEY_BITS = 2048;
export const PASS_KEY_BYTES = PASS_KEY_BITS / 8;
export const MAX_PASSES_PER_REQUEST = 10;
export const PASS_BURST = 30;
export const PASS_REFILL_MS = 2 * 60 * 1000;
export const EPOCH_DAYS = 7;
const DAY_MS = 24 * 60 * 60 * 1000;
const KEY_MODEL = "commyPassKey";
const BUCKET_MODEL = "commyPassBucket";
const KEYS_CACHE_SECONDS = 3600;
const BASE64URL = /^[A-Za-z0-9_-]+$/;
const makeKeyPair = promisify(generateKeyPair);

export const epochOf = (ms) => Math.floor(Math.max(0, ms) / DAY_MS / EPOCH_DAYS);

export function refill(bucket, now, { burst = PASS_BURST, refillMs = PASS_REFILL_MS } = {}) {
  if (!bucket) return burst;
  const earned = (now - new Date(bucket.updatedAt).getTime()) / refillMs;
  return Math.min(burst, Math.max(0, Number(bucket.tokens) + Math.max(0, earned)));
}

export function blindedMessage(text) {
  if (typeof text !== "string" || !BASE64URL.test(text)) return null;
  const bytes = Buffer.from(text, "base64url");
  return bytes.length === PASS_KEY_BYTES ? bytes : null;
}

export function signBlinded(privateKey, blinded) {
  return privateDecrypt({ key: privateKey, padding: constants.RSA_NO_PADDING }, blinded);
}

function oneAtATime() {
  const running = new Map();
  return (key, work) => {
    const before = running.get(key) ?? Promise.resolve();
    const mine = before.catch(() => {}).then(work);
    const settled = mine.catch(() => {});
    running.set(key, settled);
    settled.then(() => running.get(key) === settled && running.delete(key));
    return mine;
  };
}

async function freshKey(epoch, now) {
  const { publicKey, privateKey } = await makeKeyPair("rsa", { modulusLength: PASS_KEY_BITS, publicExponent: 0x10001 });
  return {
    epoch,
    publicKey: publicKey.export({ type: "spki", format: "der" }).toString("base64url"),
    privateKey: privateKey.export({ type: "pkcs8", format: "pem" }),
    createdAt: new Date(now),
  };
}

export function createPassKeys({ now = () => Date.now(), makeKey = freshKey } = {}) {
  const known = new Map();
  const serial = oneAtATime();
  let prunedFor = null;

  function remember(row) {
    const held = { epoch: Number(row.epoch), der: row.publicKey, privateKey: createPrivateKey(row.privateKey) };
    if (createPublicKey(held.privateKey).export({ type: "spki", format: "der" }).toString("base64url") !== held.der) {
      throw new Error(`the stored pass key for week ${held.epoch} does not match its public half`);
    }
    known.set(held.epoch, held);
    return held;
  }

  async function find(adapter, epoch) {
    if (known.has(epoch)) return known.get(epoch);
    const row = await adapter.findOne({ model: KEY_MODEL, where: [{ field: "epoch", value: epoch }] });
    return row ? remember(row) : null;
  }

  function ensure(adapter, epoch) {
    return serial(`key:${epoch}`, async () => {
      const found = await find(adapter, epoch);
      if (found) return found;
      const made = await makeKey(epoch, now());
      try {
        return remember(await adapter.create({ model: KEY_MODEL, data: made }));
      } catch (err) {
        const raced = await find(adapter, epoch);
        if (raced) return raced;
        throw err;
      }
    });
  }

  return {
    current: () => epochOf(now()),
    ensure,
    async published(adapter) {
      const epoch = epochOf(now());
      if (prunedFor !== epoch) {
        prunedFor = epoch;
        await this.prune(adapter).catch((err) => console.warn(`[auth] pruning old pass keys failed: ${err.message}`));
      }
      const older = await find(adapter, epoch - 1);
      const keys = [older, await ensure(adapter, epoch), await ensure(adapter, epoch + 1)].filter(Boolean);
      return keys.map((key) => ({ epoch: key.epoch, der: key.der }));
    },
    async prune(adapter) {
      const keep = epochOf(now()) - 1;
      for (const epoch of known.keys()) if (epoch < keep) known.delete(epoch);
      await adapter.deleteMany({ model: KEY_MODEL, where: [{ field: "epoch", operator: "lt", value: keep }] });
    },
  };
}

export function createPassBuckets({ now = () => Date.now(), burst = PASS_BURST, refillMs = PASS_REFILL_MS } = {}) {
  const serial = oneAtATime();
  return {
    take(adapter, userId, wanted) {
      return serial(`bucket:${userId}`, async () => {
        const at = now();
        const row = await adapter.findOne({ model: BUCKET_MODEL, where: [{ field: "userId", value: userId }] });
        const tokens = refill(row, at, { burst, refillMs });
        const granted = Math.min(wanted, Math.floor(tokens));
        const update = { tokens: tokens - granted, updatedAt: new Date(at) };
        if (row) await adapter.update({ model: BUCKET_MODEL, where: [{ field: "id", value: row.id }], update });
        else await adapter.create({ model: BUCKET_MODEL, data: { userId, ...update } });
        const waitMs = granted ? 0 : Math.ceil((1 - (tokens - granted)) * refillMs);
        return { granted, waitMs };
      });
    },
  };
}

export const commyPasses = ({ abuseContact = null, keys = createPassKeys(), buckets = createPassBuckets() } = {}) => ({
  id: "commy-passes",
  schema: {
    [KEY_MODEL]: {
      fields: {
        epoch: { type: "number", required: true, unique: true },
        publicKey: { type: "string", required: true },
        privateKey: { type: "string", required: true },
        createdAt: { type: "date", required: true },
      },
    },
    [BUCKET_MODEL]: {
      fields: {
        userId: {
          type: "string",
          required: true,
          unique: true,
          references: { model: "user", field: "id", onDelete: "cascade" },
        },
        tokens: { type: "number", required: true },
        updatedAt: { type: "date", required: true },
      },
    },
  },
  endpoints: {
    commyPassKeys: createAuthEndpoint(PASS_KEYS_PATH, { method: "GET" }, async (ctx) => {
      const published = await keys.published(ctx.context.adapter);
      ctx.setHeader("cache-control", `public, max-age=${KEYS_CACHE_SECONDS}`);
      return ctx.json({ keys: published });
    }),

    commyPassIssue: createAuthEndpoint(
      PASS_ISSUE_PATH,
      {
        method: "POST",
        use: [sessionMiddleware],
        body: z.object({ epoch: z.number().int(), blinded: z.array(z.string()).min(1).max(MAX_PASSES_PER_REQUEST) }),
      },
      async (ctx) => {
        const userId = ctx.context.session.user.id;
        if (isSuspended(await ctx.context.internalAdapter.findUserById(userId))) throw suspendedRefusal(abuseContact);
        if (ctx.body.epoch !== keys.current()) throw new APIError("BAD_REQUEST", { message: "Passes are only made for the current week", code: "WRONG_WEEK" });
        const blinded = ctx.body.blinded.map(blindedMessage);
        if (blinded.some((b) => !b)) throw new APIError("BAD_REQUEST", { message: `Each blinded pass must be ${PASS_KEY_BYTES} bytes of base64url` });
        const key = await keys.ensure(ctx.context.adapter, ctx.body.epoch);
        let signatures;
        try {
          signatures = blinded.map((message) => signBlinded(key.privateKey, message).toString("base64url"));
        } catch {
          throw new APIError("BAD_REQUEST", { message: "A blinded pass does not fit this week's key" });
        }
        const { granted, waitMs } = await buckets.take(ctx.context.adapter, userId, blinded.length);
        if (!granted) {
          ctx.setHeader("retry-after", String(Math.max(1, Math.ceil(waitMs / 1000))));
          throw new APIError("TOO_MANY_REQUESTS", { message: "No new server passes are left for now", code: "NO_PASSES_LEFT" });
        }
        ctx.setHeader("cache-control", "no-store");
        return ctx.json({ epoch: key.epoch, signatures: signatures.slice(0, granted) });
      },
    ),
  },
});
