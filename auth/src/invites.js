// Shareable invite links: https://<server>/i/<code>#<key> maps a short code to
// a Commy server ticket, so the link always resolves on the server that made it.
// The app seals the ticket with <key> before sending it, and the key only
// travels in the link's #fragment, which never reaches a server. So this server
// stores only the code, the sealed ticket and an expiry: it cannot read which
// Commy server an invite opens, its name, or the device that shared it.
// Rows from before sealing hold a plain ticket and resolve until they expire.
import { randomInt } from "node:crypto";
import * as z from "zod";
import { APIError, createAuthEndpoint, getSessionFromCtx } from "better-auth/api";

const MODEL = "invite";
const DAY_MS = 24 * 60 * 60 * 1000;
export const CODE_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
export const CODE_LENGTH = 8;
export const MAX_TICKET_LENGTH = 8192;
const SEALED = /^sealed1\.[A-Za-z0-9_-]{40,}$/;
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

// Only sealed tickets are accepted; a plain one would tell this server which
// Commy server the invite opens.
export function validTicket(value) {
  const ticket = String(value ?? "").trim();
  return ticket.length <= MAX_TICKET_LENGTH && SEALED.test(ticket) ? ticket : null;
}

export function isPlainTicket(value) {
  return /^commy[a-z2-7]{40,}$/.test(String(value ?? "").trim());
}

// Anonymous links are short lived; signed in users may keep them longer.
export function clampDays(value, signedIn) {
  const max = signedIn ? 30 : 7;
  const days = Number.isFinite(value) ? Math.round(value) : 7;
  return Math.min(Math.max(days, 1), max);
}

export const commyInvites = ({ serverUrl, requireAccount = false }) => ({
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
          expiresInDays: z.number().optional(),
        }),
      },
      async (ctx) => {
        const session = await getSessionFromCtx(ctx).catch(() => null);
        if (requireAccount && !session) {
          throw new APIError("UNAUTHORIZED", { message: "Sign in to create invite links on this server" });
        }
        if (isPlainTicket(ctx.body.ticket)) {
          throw new APIError("BAD_REQUEST", { message: "Update Commy to share invite links" });
        }
        const ticket = validTicket(ctx.body.ticket);
        if (!ticket) throw new APIError("BAD_REQUEST", { message: "That is not a Commy invite" });

        const adapter = ctx.context.adapter;
        const now = new Date();
        await adapter.deleteMany({ model: MODEL, where: [{ field: "expiresAt", operator: "lt", value: now }] });

        const createdBy = session?.user.id ?? null;
        const expiresAt = new Date(now.getTime() + clampDays(ctx.body.expiresInDays, Boolean(session)) * DAY_MS);
        // Sealed invites carry no name; the column predates sealing
        const serverName = "";

        // A retried request keeps one code
        const existing = await adapter.findOne({ model: MODEL, where: [{ field: "ticket", value: ticket }] });
        if (existing && (existing.createdBy ?? null) === createdBy) {
          const updated = await adapter.update({
            model: MODEL,
            where: [{ field: "id", value: existing.id }],
            update: { expiresAt },
          });
          return ctx.json(publicInvite(updated ?? { ...existing, expiresAt }, serverUrl));
        }

        for (let attempt = 0; attempt < 5; attempt++) {
          const code = generateCode();
          const clash = await adapter.findOne({ model: MODEL, where: [{ field: "code", value: code }] });
          if (clash) continue;
          const created = await adapter.create({
            model: MODEL,
            data: { code, ticket, serverName, createdBy, createdAt: now, expiresAt },
          });
          return ctx.json(publicInvite(created, serverUrl));
        }
        throw new APIError("INTERNAL_SERVER_ERROR", { message: "Could not allocate an invite code" });
      },
    ),

    resolveCommyInvite: createAuthEndpoint(
      "/commy/invites/resolve",
      { method: "GET", query: z.object({ code: z.string() }) },
      async (ctx) => {
        const code = normalizeCode(ctx.query.code);
        const row = code
          ? await ctx.context.adapter.findOne({ model: MODEL, where: [{ field: "code", value: code }] })
          : null;
        if (!row || new Date(row.expiresAt) < new Date()) {
          throw new APIError("NOT_FOUND", { message: "This invite is invalid or has expired" });
        }
        return ctx.json({ ...publicInvite(row, serverUrl), ticket: row.ticket });
      },
    ),

    revokeCommyInvite: createAuthEndpoint(
      "/commy/invites/revoke",
      { method: "POST", body: z.object({ code: z.string() }) },
      async (ctx) => {
        const session = await getSessionFromCtx(ctx);
        if (!session) throw new APIError("UNAUTHORIZED", { message: "Sign in to revoke invite links" });
        const code = normalizeCode(ctx.body.code);
        if (code) {
          await ctx.context.adapter.deleteMany({
            model: MODEL,
            where: [
              { field: "code", value: code },
              { field: "createdBy", value: session.user.id },
            ],
          });
        }
        return ctx.json({ success: true });
      },
    ),
  },
});

// The url has no #key: only the app that sealed the ticket can add it.
// serverName is only set for plain rows from before sealing.
export function publicInvite(row, serverUrl) {
  return {
    code: row.code,
    url: `${serverUrl}/i/${row.code}`,
    serverName: row.serverName || null,
    sealed: SEALED.test(row.ticket ?? ""),
    expiresAt: row.expiresAt,
  };
}
