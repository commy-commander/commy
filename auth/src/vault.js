// Sealed per-account storage. Each account keeps a few named items, such as
// "servers" (the list of Commy servers it is in). The apps seal every item with
// a key that only the account's own devices hold, so this server stores bytes
// it cannot read: it never learns which servers someone is in. Writes carry the
// version they replace, so two devices cannot silently overwrite each other.
import * as z from "zod";
import { APIError, createAuthEndpoint, sessionMiddleware } from "better-auth/api";

const MODEL = "vaultItem";
export const MAX_VAULT_BYTES = 256 * 1024;
const NAME = /^[a-z0-9-]{1,32}$/;
// Same format as commy-net's sealed.rs: sealed1.<base64url(nonce || ciphertext)>
const SEALED = /^sealed1\.[A-Za-z0-9_-]{40,}$/;

export function normalizeVaultName(value) {
  const name = String(value ?? "").trim();
  return NAME.test(name) ? name : null;
}

export function validSealed(value) {
  const data = String(value ?? "");
  return data.length <= MAX_VAULT_BYTES && SEALED.test(data) ? data : null;
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

export const commyVault = () => ({
  id: "commy-vault",
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
        body: z.object({ name: z.string(), data: z.string(), version: z.number() }),
      },
      async (ctx) => {
        const name = normalizeVaultName(ctx.body.name);
        if (!name) throw new APIError("BAD_REQUEST", { message: "Unknown vault item" });
        const data = validSealed(ctx.body.data);
        if (!data) throw new APIError("BAD_REQUEST", { message: "Vault items must be sealed and under 256 KiB" });
        const userId = ctx.context.session.user.id;
        const adapter = ctx.context.adapter;
        const now = new Date();
        const existing = await findItem(adapter, userId, name);
        const current = existing?.version ?? 0;
        if (ctx.body.version !== current) {
          // The caller merges the newer item and tries again
          throw conflict(name, existing);
        }
        if (existing) {
          const updated = await adapter.update({
            model: MODEL,
            where: [
              { field: "id", value: existing.id },
              { field: "version", value: current },
            ],
            update: { data, version: current + 1, updatedAt: now },
          });
          if (!updated) {
            throw conflict(name, await findItem(adapter, userId, name));
          }
          return ctx.json(publicItem(name, updated));
        }
        const created = await adapter.create({
          model: MODEL,
          data: { userId, name, data, version: 1, updatedAt: now },
        });
        return ctx.json(publicItem(name, created));
      },
    ),

    deleteCommyVaultItem: createAuthEndpoint(
      "/commy/vault/delete",
      { method: "POST", use: [sessionMiddleware], body: z.object({ name: z.string() }) },
      async (ctx) => {
        const name = normalizeVaultName(ctx.body.name);
        if (name) {
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
