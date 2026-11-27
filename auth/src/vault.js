import * as z from "zod";
import { APIError, createAuthEndpoint, sessionMiddleware } from "better-auth/api";

const MODEL = "vaultItem";
const EARLIER = "vaultEarlier";
export const MAX_VAULT_BYTES = 256 * 1024;
export const MAX_WRITER = 80;
export const KEPT_EARLIER_NAMES = new Set(["servers"]);
export const NO_LONGER_KEPT = new Set(["entry-key"]);
export const NEWEST_EARLIER_KEPT = 20;
export const EARLIER_DAYS = 45;
const DAY_MS = 24 * 60 * 60 * 1000;
const NAME = /^[a-z0-9-]{1,32}$/;
export const COMMY_NET_SEALED_FORMAT = /^sealed1\.[A-Za-z0-9_-]{40,}$/;

export function normalizeVaultName(value) {
  const name = String(value ?? "").trim();
  return NAME.test(name) ? name : null;
}

export function validSealed(value) {
  const data = String(value ?? "");
  return data.length <= MAX_VAULT_BYTES && COMMY_NET_SEALED_FORMAT.test(data) ? data : null;
}

export function cleanWriter(value) {
  const text = String(value ?? "").replace(/[^\x20-\x7e]/g, "").trim().slice(0, MAX_WRITER);
  return text || null;
}

function newestFirst(rows) {
  return [...rows].sort((a, b) => new Date(b.replacedAt) - new Date(a.replacedAt) || b.version - a.version);
}

function utcDay(date) {
  return Math.floor(new Date(date).getTime() / DAY_MS);
}

export function earlierToDrop(rows, now) {
  const ordered = newestFirst(rows);
  const cutoff = now.getTime() - EARLIER_DAYS * DAY_MS;
  const firstOfDay = new Map();
  for (const row of ordered) {
    if (new Date(row.replacedAt).getTime() >= cutoff) firstOfDay.set(utcDay(row.replacedAt), row.id);
  }
  const keep = new Set([...ordered.slice(0, NEWEST_EARLIER_KEPT).map((row) => row.id), ...firstOfDay.values()]);
  return ordered.filter((row) => !keep.has(row.id)).map((row) => row.id);
}

function earlierItem(row) {
  return {
    version: row.version,
    data: row.data,
    updatedAt: row.updatedAt,
    replacedAt: row.replacedAt,
    writer: row.writer ?? null,
  };
}

async function findEarlier(adapter, userId, name) {
  return adapter.findMany({
    model: EARLIER,
    where: [
      { field: "userId", value: userId },
      { field: "name", value: name },
    ],
  });
}

async function keepEarlierCopy(adapter, row, now) {
  if (!row || !KEPT_EARLIER_NAMES.has(row.name)) return;
  await adapter.create({
    model: EARLIER,
    data: {
      userId: row.userId,
      name: row.name,
      data: row.data,
      version: row.version,
      updatedAt: row.updatedAt,
      replacedAt: now,
      writer: row.writer ?? null,
    },
  });
  const drop = earlierToDrop(await findEarlier(adapter, row.userId, row.name), now);
  if (drop.length) await adapter.deleteMany({ model: EARLIER, where: [{ field: "id", operator: "in", value: drop }] });
}

function publicItem(name, row) {
  return {
    name,
    data: row?.data ?? null,
    version: row?.version ?? 0,
    updatedAt: row?.updatedAt ?? null,
  };
}

function conflict(name, row) {
  return new APIError("CONFLICT", { message: "Changed on another device", conflict: true, ...publicItem(name, row) });
}

async function findItem(adapter, userId, name) {
  return adapter.findOne({
    model: MODEL,
    where: [
      { field: "userId", value: userId },
      { field: "name", value: name },
    ],
  });
}

export async function forgetItemsNoLongerKept(adapter) {
  for (const model of [MODEL, EARLIER]) {
    await adapter.deleteMany({ model, where: [{ field: "name", operator: "in", value: [...NO_LONGER_KEPT] }] });
  }
}

export const commyVault = () => ({
  id: "commy-vault",
  init(context) {
    forgetItemsNoLongerKept(context.adapter).catch((err) => console.warn(`[auth] could not clear vault items no longer kept: ${err.message}`));
  },
  schema: {
    [MODEL]: {
      fields: {
        userId: {
          type: "string",
          required: true,
          references: { model: "user", field: "id", onDelete: "cascade" },
          index: true,
        },
        name: { type: "string", required: true },
        data: { type: "string", required: true },
        version: { type: "number", required: true },
        updatedAt: { type: "date", required: true },
        writer: { type: "string", required: false },
      },
    },
    [EARLIER]: {
      fields: {
        userId: {
          type: "string",
          required: true,
          references: { model: "user", field: "id", onDelete: "cascade" },
          index: true,
        },
        name: { type: "string", required: true },
        data: { type: "string", required: true },
        version: { type: "number", required: true },
        updatedAt: { type: "date", required: true },
        replacedAt: { type: "date", required: true },
        writer: { type: "string", required: false },
      },
    },
  },
  endpoints: {
    getCommyVaultItem: createAuthEndpoint(
      "/commy/vault",
      { method: "GET", use: [sessionMiddleware], query: z.object({ name: z.string() }) },
      async (ctx) => {
        const name = normalizeVaultName(ctx.query.name);
        if (!name) throw new APIError("BAD_REQUEST", { message: "Unknown vault item" });
        const row = await findItem(ctx.context.adapter, ctx.context.session.user.id, name);
        return ctx.json(publicItem(name, row));
      },
    ),

    putCommyVaultItem: createAuthEndpoint(
      "/commy/vault/put",
      {
        method: "POST",
        use: [sessionMiddleware],
        body: z.object({ name: z.string(), data: z.string(), version: z.number(), writer: z.string().optional() }),
      },
      async (ctx) => {
        const name = normalizeVaultName(ctx.body.name);
        if (!name) throw new APIError("BAD_REQUEST", { message: "Unknown vault item" });
        if (NO_LONGER_KEPT.has(name)) throw new APIError("BAD_REQUEST", { message: "This item is no longer kept with your account" });
        const data = validSealed(ctx.body.data);
        if (!data) throw new APIError("BAD_REQUEST", { message: "Vault items must be sealed and under 256 KiB" });
        const userId = ctx.context.session.user.id;
        const adapter = ctx.context.adapter;
        const now = new Date();
        const writer = cleanWriter(ctx.body.writer);
        const existing = await findItem(adapter, userId, name);
        const current = existing?.version ?? 0;
        if (ctx.body.version !== current) {
          throw conflict(name, existing);
        }
        if (existing) {
          const updated = await adapter.update({
            model: MODEL,
            where: [
              { field: "id", value: existing.id },
              { field: "version", value: current },
            ],
            update: { data, version: current + 1, updatedAt: now, writer },
          });
          if (!updated) {
            throw conflict(name, await findItem(adapter, userId, name));
          }
          await keepEarlierCopy(adapter, existing, now);
          return ctx.json(publicItem(name, updated));
        }
        const created = await adapter.create({
          model: MODEL,
          data: { userId, name, data, version: 1, updatedAt: now, writer },
        });
        return ctx.json(publicItem(name, created));
      },
    ),

    getCommyVaultEarlier: createAuthEndpoint(
      "/commy/vault/earlier",
      { method: "GET", use: [sessionMiddleware], query: z.object({ name: z.string() }) },
      async (ctx) => {
        const name = normalizeVaultName(ctx.query.name);
        if (!name || !KEPT_EARLIER_NAMES.has(name)) throw new APIError("BAD_REQUEST", { message: "No earlier copies are kept of that item" });
        const rows = await findEarlier(ctx.context.adapter, ctx.context.session.user.id, name);
        const items = newestFirst(rows).map(earlierItem);
        return ctx.json({ name, items });
      },
    ),

    deleteCommyVaultItem: createAuthEndpoint(
      "/commy/vault/delete",
      { method: "POST", use: [sessionMiddleware], body: z.object({ name: z.string() }) },
      async (ctx) => {
        const name = normalizeVaultName(ctx.body.name);
        if (name) {
          await keepEarlierCopy(ctx.context.adapter, await findItem(ctx.context.adapter, ctx.context.session.user.id, name), new Date());
          await ctx.context.adapter.deleteMany({
            model: MODEL,
            where: [
              { field: "userId", value: ctx.context.session.user.id },
              { field: "name", value: name },
            ],
          });
        }
        return ctx.json({ success: true });
      },
    ),
  },
});
