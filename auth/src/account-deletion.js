import * as z from "zod";
import { createHmac, randomInt, timingSafeEqual } from "node:crypto";
import { APIError, createAuthEndpoint, sessionMiddleware } from "better-auth/api";
import { deleteSessionCookie } from "better-auth/cookies";
import { removeInvite } from "./invites.js";

export const DELETE_CODE_DIGITS = 6;
export const DELETE_CODE_MINUTES = 15;
export const DELETE_CODE_TRIES = 5;
const MAX_PASSWORD = 128;
const OWNED_ROWS = [
  { model: "linkedDevice", field: "userId" },
  { model: "vaultItem", field: "userId" },
  { model: "passkey", field: "userId" },
  { model: "twoFactor", field: "userId" },
  { model: "deviceCode", field: "userId" },
  { model: "directoryListing", field: "createdBy" },
];

const codeIdentifier = (userId) => `commy-delete-account-${userId}`;

export function newDeleteCode() {
  return String(randomInt(0, 10 ** DELETE_CODE_DIGITS)).padStart(DELETE_CODE_DIGITS, "0");
}

export function cleanCode(value) {
  const digits = String(value ?? "").replace(/[\s-]/g, "");
  return new RegExp(`^\\d{${DELETE_CODE_DIGITS}}$`).test(digits) ? digits : null;
}

function codeHash(secret, userId, code) {
  return createHmac("sha256", secret).update(`delete-account\n${userId}\n${code}`).digest("base64url");
}

function sameHash(a, b) {
  const left = Buffer.from(String(a));
  const right = Buffer.from(String(b));
  return left.length === right.length && timingSafeEqual(left, right);
}

function refuse(code, message) {
  return new APIError("BAD_REQUEST", { code, message });
}

async function hasPassword(ctx, userId) {
  const account = await ctx.context.internalAdapter.findCredentialAccount(userId);
  return Boolean(account?.password);
}

async function storeCode(ctx, userId, code) {
  const internal = ctx.context.internalAdapter;
  const identifier = codeIdentifier(userId);
  await internal.deleteVerificationByIdentifier(identifier);
  await internal.createVerificationValue({
    identifier,
    value: JSON.stringify({ hash: codeHash(ctx.context.secret, userId, code), tries: 0 }),
    expiresAt: new Date(Date.now() + DELETE_CODE_MINUTES * 60 * 1000),
  });
}

async function codeMatches(ctx, userId, code) {
  const internal = ctx.context.internalAdapter;
  const identifier = codeIdentifier(userId);
  const row = await internal.findVerificationValue(identifier);
  if (!row || new Date(row.expiresAt) < new Date()) return false;
  let stored;
  try {
    stored = JSON.parse(row.value);
  } catch {
    stored = null;
  }
  if (!stored || stored.tries >= DELETE_CODE_TRIES) {
    await internal.deleteVerificationByIdentifier(identifier);
    return false;
  }
  if (sameHash(stored.hash, codeHash(ctx.context.secret, userId, code))) return true;
  await internal.deleteVerificationByIdentifier(identifier);
  if (stored.tries + 1 < DELETE_CODE_TRIES) {
    await internal.createVerificationValue({
      identifier,
      value: JSON.stringify({ ...stored, tries: stored.tries + 1 }),
      expiresAt: new Date(row.expiresAt),
    });
  }
  return false;
}

async function passwordMatches(ctx, userId, password) {
  const account = await ctx.context.internalAdapter.findCredentialAccount(userId);
  if (!account?.password) return false;
  return ctx.context.password.verify({ hash: account.password, password });
}

function isFresh(ctx) {
  const freshAge = ctx.context.sessionConfig.freshAge;
  if (freshAge === 0) return true;
  return Date.now() - new Date(ctx.context.session.session.createdAt).getTime() < freshAge * 1000;
}

async function confirmed(ctx, userId, { emailCodes }) {
  const { code, password } = ctx.body;
  if (password) {
    if (await passwordMatches(ctx, userId, password)) return;
    throw refuse("INVALID_PASSWORD", "That password is not right.");
  }
  if (code !== undefined) {
    const digits = cleanCode(code);
    if (digits && emailCodes && (await codeMatches(ctx, userId, digits))) return;
    throw refuse("INVALID_CODE", "That code is not right or has expired. Ask for a new one.");
  }
  if (!emailCodes && isFresh(ctx)) return;
  throw refuse("CONFIRMATION_REQUIRED", "Confirm with the code we emailed you or your password.");
}

export async function eraseAccount(adapter, internal, userId) {
  const invites = await adapter.findMany({ model: "invite", where: [{ field: "createdBy", value: userId }] });
  for (const invite of invites) await removeInvite(adapter, invite.code);
  for (const { model, field } of OWNED_ROWS) {
    await adapter.deleteMany({ model, where: [{ field, value: userId }] });
  }
  await internal.deleteVerificationByIdentifier(codeIdentifier(userId));
  await internal.deleteUser(userId);
  await internal.deleteUserSessions(userId);
}

function warnMailFailed(err) {
  console.warn(`[mail] account deletion mail failed: ${err?.code ?? err?.message ?? err}`);
}

export const commyAccountDeletion = ({ instanceName, send, emailCodes }) => ({
  id: "commy-account-deletion",
  endpoints: {
    startCommyAccountDeletion: createAuthEndpoint(
      "/commy/account/delete/start",
      { method: "POST", use: [sessionMiddleware] },
      async (ctx) => {
        const user = ctx.context.session.user;
        const password = await hasPassword(ctx, user.id);
        if (!emailCodes) return ctx.json({ confirm: password ? "password" : "fresh", password, email: null });
        const code = newDeleteCode();
        await storeCode(ctx, user.id, code);
        await send({
          to: user.email,
          subject: `Your code to delete your ${instanceName} account`,
          heading: "Delete your account",
          body: `Someone asked to delete this account for good. Enter this code to confirm:\n\n${code}\n\nThe code expires in ${DELETE_CODE_MINUTES} minutes. If this was not you, ignore this email, nothing changes, and consider changing your password.`,
        });
        return ctx.json({ confirm: "email", password, email: user.email });
      },
    ),
    finishCommyAccountDeletion: createAuthEndpoint(
      "/commy/account/delete/finish",
      {
        method: "POST",
        use: [sessionMiddleware],
        body: z.object({
          code: z.string().max(20).optional(),
          password: z.string().max(MAX_PASSWORD).optional(),
        }),
      },
      async (ctx) => {
        const user = ctx.context.session.user;
        await confirmed(ctx, user.id, { emailCodes });
        await eraseAccount(ctx.context.adapter, ctx.context.internalAdapter, user.id);
        deleteSessionCookie(ctx);
        if (emailCodes && user.email) {
          await send({
            to: user.email,
            subject: `Your ${instanceName} account was deleted`,
            heading: "Your account was deleted",
            body: "Your account, sign-ins, linked devices, passkeys, invite links and Discover listings are gone. Your apps were signed out. This is the last email we will send to this address.",
          }).catch(warnMailFailed);
        }
        return ctx.json({ deleted: true });
      },
    ),
  },
});
