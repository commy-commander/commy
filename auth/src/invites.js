import { createHash, randomInt, timingSafeEqual } from "node:crypto";
import * as z from "zod";
import { APIError, createAuthEndpoint, getSessionFromCtx } from "better-auth/api";
import { COMMY_NET_SEALED_FORMAT } from "./vault.js";

const MODEL = "invite";
const BACKGROUND_MODEL = "inviteBackground";
const HOUR_MS = 60 * 60 * 1000;
const DAY_MS = 24 * HOUR_MS;
export const EXPIRY_CHOICES = { "1h": HOUR_MS, "1d": DAY_MS, "7d": 7 * DAY_MS, "30d": 30 * DAY_MS, never: null };
export const NEVER = new Date("9999-12-31T23:59:59.000Z");
export const DEFAULT_MAX_PERMANENT = 20000;
export const CODE_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
export const CODE_LENGTH = 8;
export const MAX_TICKET_LENGTH = 8192;
export const MAX_BACKGROUND_LENGTH = 600_000;
export const MAX_CUSTOM_PER_ACCOUNT = 10;
export const NAME_MIN = 3;
export const NAME_MAX = 32;
export const RESERVED_NAMES = new Set([
  "admin", "commy", "support", "help", "staff", "official", "moderator", "mod", "team", "security", "login",
  "account", "download", "docs", "privacy", "invite", "invites",
]);
const REVOKE_HASH = /^[0-9a-f]{64}$/;
const NAME = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const OLDER_APP_ANONYMOUS_MAX_DAYS = 7;
const OLDER_APP_SIGNED_IN_MAX_DAYS = 30;
const SEALED_INVITES_STORE_NO_SERVER_NAME = "";
const CODE = new RegExp(`^[${CODE_ALPHABET}]{${CODE_LENGTH}}$`);

export function generateCode() {
  let code = "";
  for (let i = 0; i < CODE_LENGTH; i++) code += CODE_ALPHABET[randomInt(CODE_ALPHABET.length)];
  return code;
}

export function normalizeCode(value) {
  const code = String(value ?? "").trim().toUpperCase();
  return CODE.test(code) ? code : null;
}

export function normalizeName(value) {
  const name = String(value ?? "").trim().toLowerCase();
  const looksLikeCode = name.length === CODE_LENGTH && !name.includes("-");
  return name.length >= NAME_MIN && name.length <= NAME_MAX && NAME.test(name) && !looksLikeCode ? name : null;
}

export function customName(value) {
  const name = normalizeName(value);
  return name && !RESERVED_NAMES.has(name) ? name : null;
}

export function normalizeLinkCode(value) {
  return normalizeCode(value) ?? normalizeName(value);
}

export function hashRevokeSecret(secret) {
  return createHash("sha256").update(String(secret)).digest("hex");
}

export function revokeSecretMatches(secret, hash) {
  if (!secret || !REVOKE_HASH.test(hash ?? "")) return false;
  return timingSafeEqual(Buffer.from(hashRevokeSecret(secret)), Buffer.from(hash));
}

export function validBackground(value) {
  const background = String(value ?? "").trim();
  return background.length <= MAX_BACKGROUND_LENGTH && COMMY_NET_SEALED_FORMAT.test(background) ? background : null;
}

export function validTicket(value) {
  const ticket = String(value ?? "").trim();
  return ticket.length <= MAX_TICKET_LENGTH && COMMY_NET_SEALED_FORMAT.test(ticket) ? ticket : null;
}

export function isPlainTicket(value) {
  return /^commy[a-z2-7]{40,}$/.test(String(value ?? "").trim());
}

export function clampDays(value, signedIn) {
  const max = signedIn ? OLDER_APP_SIGNED_IN_MAX_DAYS : OLDER_APP_ANONYMOUS_MAX_DAYS;
  const days = Number.isFinite(value) ? Math.round(value) : OLDER_APP_ANONYMOUS_MAX_DAYS;
  return Math.min(Math.max(days, 1), max);
}

export function expiryFor(choice, now = new Date()) {
  if (!Object.hasOwn(EXPIRY_CHOICES, choice)) return null;
  const ms = EXPIRY_CHOICES[choice];
  return ms === null ? NEVER : new Date(now.getTime() + ms);
}

export function isPermanent(expiresAt) {
  return new Date(expiresAt).getTime() >= NEVER.getTime();
}

export const commyInvites = ({ serverUrl, requireAccount = false, maxPermanent = DEFAULT_MAX_PERMANENT }) => ({
  id: "commy-invites",
  schema: {
    [MODEL]: {
      fields: {
        code: { type: "string", required: true, unique: true },
        ticket: { type: "string", required: true, index: true },
        serverName: { type: "string", required: true },
        createdBy: { type: "string", required: false, index: true },
        createdAt: { type: "date", required: true },
        expiresAt: { type: "date", required: true, index: true },
        revokeHash: { type: "string", required: false },
        custom: { type: "boolean", required: false },
      },
    },
    [BACKGROUND_MODEL]: {
      fields: {
        code: { type: "string", required: true, unique: true },
        data: { type: "string", required: true },
        expiresAt: { type: "date", required: true, index: true },
      },
    },
  },
  endpoints: {
    createCommyInvite: createAuthEndpoint(
      "/commy/invites/create",
      {
        method: "POST",
        body: z.object({
          ticket: z.string(),
          expiresIn: z.enum(Object.keys(EXPIRY_CHOICES)).optional(),
          expiresInDays: z.number().optional(),
          revokeHash: z.string().optional(),
          code: z.string().optional(),
          background: z.string().optional(),
        }),
      },
      async (ctx) => {
        const session = await getSessionFromCtx(ctx).catch(() => null);
        if (requireAccount && !session) {
          throw new APIError("UNAUTHORIZED", { message: "Sign in to create invite links on this server" });
        }
        if (ctx.body.expiresIn === "never" && !session) {
          throw new APIError("UNAUTHORIZED", { message: "Sign in to create invite links that never expire" });
        }
        if (isPlainTicket(ctx.body.ticket)) {
          throw new APIError("BAD_REQUEST", { message: "Update Commy to share invite links" });
        }
        const ticket = validTicket(ctx.body.ticket);
        if (!ticket) throw new APIError("BAD_REQUEST", { message: "That is not a Commy invite" });
        const revokeHash = ctx.body.revokeHash === undefined ? null : ctx.body.revokeHash.toLowerCase();
        if (revokeHash !== null && !REVOKE_HASH.test(revokeHash)) {
          throw new APIError("BAD_REQUEST", { message: "That revoke key is not valid" });
        }
        const wantsName = ctx.body.code !== undefined;
        const name = wantsName ? customName(ctx.body.code) : null;
        if (wantsName && !name) {
          throw new APIError("BAD_REQUEST", { message: "That custom name is not allowed. Pick another one." });
        }
        const background = ctx.body.background === undefined ? null : validBackground(ctx.body.background);
        if (ctx.body.background !== undefined && !background) {
          throw new APIError("BAD_REQUEST", { message: "That invite background is not valid" });
        }
        if ((name || background) && !session) {
          throw new APIError("UNAUTHORIZED", { message: "Sign in to pick a custom name or background for invite links" });
        }

        const adapter = ctx.context.adapter;
        const now = new Date();
        await adapter.deleteMany({ model: MODEL, where: [{ field: "expiresAt", operator: "lt", value: now }] });
        await adapter.deleteMany({ model: BACKGROUND_MODEL, where: [{ field: "expiresAt", operator: "lt", value: now }] });

        const createdBy = session?.user.id ?? null;
        const expiresAt = ctx.body.expiresIn
          ? expiryFor(ctx.body.expiresIn, now)
          : new Date(now.getTime() + clampDays(ctx.body.expiresInDays, Boolean(session)) * DAY_MS);
        if (isPermanent(expiresAt)) {
          const permanent = await adapter.count({
            model: MODEL,
            where: [{ field: "expiresAt", operator: "gte", value: NEVER }],
          });
          if (permanent >= maxPermanent) {
            throw new APIError("FORBIDDEN", {
              message: "This server holds too many links that never expire. Pick an expiry instead.",
            });
          }
        }
        const extra = revokeHash ? { revokeHash } : {};
        const storeBackground = async (code) => {
          await adapter.deleteMany({ model: BACKGROUND_MODEL, where: [{ field: "code", value: code }] });
          if (background) await adapter.create({ model: BACKGROUND_MODEL, data: { code, data: background, expiresAt } });
        };

        if (name) {
          const taken = await adapter.findOne({ model: MODEL, where: [{ field: "code", value: name }] });
          if (taken && taken.createdBy !== createdBy) {
            throw new APIError("CONFLICT", { message: "That name is taken. Pick another one." });
          }
          if (!taken) {
            const mine = await adapter.count({
              model: MODEL,
              where: [
                { field: "createdBy", value: createdBy },
                { field: "custom", value: true },
              ],
            });
            if (mine >= MAX_CUSTOM_PER_ACCOUNT) {
              throw new APIError("FORBIDDEN", { message: `You can hold up to ${MAX_CUSTOM_PER_ACCOUNT} custom invite names. Revoke one first.` });
            }
          }
          const fields = { ticket, expiresAt, createdAt: now, custom: true, revokeHash: revokeHash ?? null };
          const row = taken
            ? (await adapter.update({ model: MODEL, where: [{ field: "id", value: taken.id }], update: fields })) ?? { ...taken, ...fields }
            : await adapter.create({
                model: MODEL,
                data: { code: name, serverName: SEALED_INVITES_STORE_NO_SERVER_NAME, createdBy, ...fields },
              });
          await storeBackground(name);
          return ctx.json(publicInvite(row, serverUrl));
        }

        const existing = await adapter.findOne({ model: MODEL, where: [{ field: "ticket", value: ticket }] });
        if (existing && !existing.custom && (existing.createdBy ?? null) === createdBy) {
          const updated = await adapter.update({
            model: MODEL,
            where: [{ field: "id", value: existing.id }],
            update: { expiresAt, ...extra },
          });
          await storeBackground(existing.code);
          return ctx.json(publicInvite(updated ?? { ...existing, expiresAt, ...extra }, serverUrl));
        }

        for (let attempt = 0; attempt < 5; attempt++) {
          const code = generateCode();
          const clash = await adapter.findOne({ model: MODEL, where: [{ field: "code", value: code }] });
          if (clash) continue;
          const created = await adapter.create({
            model: MODEL,
            data: { code, ticket, serverName: SEALED_INVITES_STORE_NO_SERVER_NAME, createdBy, createdAt: now, expiresAt, ...extra },
          });
          await storeBackground(code);
          return ctx.json(publicInvite(created, serverUrl));
        }
        throw new APIError("INTERNAL_SERVER_ERROR", { message: "Could not allocate an invite code" });
      },
    ),

    resolveCommyInvite: createAuthEndpoint(
      "/commy/invites/resolve",
      { method: "GET", query: z.object({ code: z.string() }) },
      async (ctx) => {
        const row = await liveInvite(ctx.context.adapter, ctx.query.code);
        const background = await ctx.context.adapter.findOne({
          model: BACKGROUND_MODEL,
          where: [{ field: "code", value: row.code }],
        });
        return ctx.json({ ...publicInvite(row, serverUrl), ticket: row.ticket, hasBackground: Boolean(background) });
      },
    ),

    commyInviteBackground: createAuthEndpoint(
      "/commy/invites/background",
      { method: "GET", query: z.object({ code: z.string() }) },
      async (ctx) => {
        const row = await liveInvite(ctx.context.adapter, ctx.query.code);
        const background = await ctx.context.adapter.findOne({
          model: BACKGROUND_MODEL,
          where: [{ field: "code", value: row.code }],
        });
        if (!background) throw new APIError("NOT_FOUND", { message: "This invite has no background" });
        return ctx.json({ background: background.data });
      },
    ),

    revokeCommyInvite: createAuthEndpoint(
      "/commy/invites/revoke",
      { method: "POST", body: z.object({ code: z.string(), secret: z.string().max(200).optional() }) },
      async (ctx) => {
        const adapter = ctx.context.adapter;
        const code = normalizeLinkCode(ctx.body.code);
        const row = code ? await adapter.findOne({ model: MODEL, where: [{ field: "code", value: code }] }) : null;
        if (ctx.body.secret === undefined) {
          const session = await getSessionFromCtx(ctx);
          if (!session) throw new APIError("UNAUTHORIZED", { message: "Sign in to revoke invite links" });
          if (row && row.createdBy === session.user.id) await removeInvite(adapter, row.code);
        } else if (row && revokeSecretMatches(ctx.body.secret, row.revokeHash)) {
          await removeInvite(adapter, row.code);
        }
        return ctx.json({ success: true });
      },
    ),
  },
});

async function liveInvite(adapter, value) {
  const code = normalizeLinkCode(value);
  const row = code ? await adapter.findOne({ model: MODEL, where: [{ field: "code", value: code }] }) : null;
  if (!row || new Date(row.expiresAt) < new Date()) {
    throw new APIError("NOT_FOUND", { message: "This invite is invalid or has expired" });
  }
  return row;
}

async function removeInvite(adapter, code) {
  await adapter.deleteMany({ model: MODEL, where: [{ field: "code", value: code }] });
  await adapter.deleteMany({ model: BACKGROUND_MODEL, where: [{ field: "code", value: code }] });
}

export function publicInvite(row, serverUrl) {
  const permanent = isPermanent(row.expiresAt);
  return {
    code: row.code,
    url: `${serverUrl}/i/${row.code}`,
    serverName: row.serverName || null,
    sealed: COMMY_NET_SEALED_FORMAT.test(row.ticket ?? ""),
    expiresAt: permanent ? null : row.expiresAt,
    permanent,
  };
}
