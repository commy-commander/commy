import * as z from "zod";
import { APIError, createAuthEndpoint, createAuthMiddleware, freshSessionMiddleware, getSessionFromCtx } from "better-auth/api";
import { deleteSessionCookie } from "better-auth/cookies";
import { generateRandomString } from "better-auth/crypto";

export const TWO_FACTOR_PAGE = "/account/two-factor";
export const SIGN_INS_THAT_REDIRECT = ["/magic-link/verify", "/callback/:id", "/verify-email"];
export const SIGN_IN_WITH_PROVIDER_TOKEN = "/sign-in/social";
const TWO_FACTOR_COOKIE = "two_factor";
const TWO_FACTOR_CHALLENGE_SECONDS = 600;
const MIN_PASSWORD = 8;
const MAX_PASSWORD = 128;

async function hasVerifiedAuthenticator(adapter, userId) {
  const row = await adapter.findOne({ model: "twoFactor", where: [{ field: "userId", value: userId }] });
  return Boolean(row && row.verified !== false);
}

export function safeReturnPath(value, baseURL) {
  if (!value) return "/account";
  try {
    const url = new URL(value, baseURL);
    if (url.origin !== new URL(baseURL).origin) return "/account";
    return `${url.pathname}${url.search}`;
  } catch {
    return "/account";
  }
}

async function startTwoFactorChallenge(ctx, userId) {
  const identifier = `2fa-${generateRandomString(20)}`;
  const expiresAt = new Date(Date.now() + TWO_FACTOR_CHALLENGE_SECONDS * 1000);
  await ctx.context.internalAdapter.createVerificationValue({ value: userId, identifier, expiresAt });
  await ctx.context.internalAdapter.createVerificationValue({ value: "0", identifier: `2fa-attempts-${identifier}`, expiresAt });
  const cookie = ctx.context.createAuthCookie(TWO_FACTOR_COOKIE, { maxAge: TWO_FACTOR_CHALLENGE_SECONDS });
  await ctx.setSignedCookie(cookie.name, identifier, ctx.context.secret, cookie.attributes);
}

export const commyTwoFactorEverywhere = ({ baseURL }) => ({
  id: "commy-two-factor-everywhere",
  hooks: {
    after: [
      {
        matcher: (ctx) => SIGN_INS_THAT_REDIRECT.includes(ctx.path),
        handler: createAuthMiddleware(async (ctx) => {
          const signedIn = ctx.context.newSession;
          if (!signedIn?.user.twoFactorEnabled) return;
          if (!(await hasVerifiedAuthenticator(ctx.context.adapter, signedIn.user.id))) return;
          deleteSessionCookie(ctx, true);
          await ctx.context.internalAdapter.deleteSession(signedIn.session.token);
          ctx.context.setNewSession(null);
          await startTwoFactorChallenge(ctx, signedIn.user.id);
          const next = safeReturnPath(ctx.context.responseHeaders?.get("location") ?? ctx.query?.callbackURL, baseURL);
          const page = new URL(TWO_FACTOR_PAGE, baseURL);
          page.searchParams.set("next", next);
          throw ctx.redirect(page.toString());
        }),
      },
      {
        matcher: (ctx) => ctx.path === SIGN_IN_WITH_PROVIDER_TOKEN,
        handler: createAuthMiddleware(async (ctx) => {
          const signedIn = ctx.context.newSession;
          if (!signedIn?.user.twoFactorEnabled) return;
          if (!(await hasVerifiedAuthenticator(ctx.context.adapter, signedIn.user.id))) return;
          deleteSessionCookie(ctx, true);
          await ctx.context.internalAdapter.deleteSession(signedIn.session.token);
          ctx.context.setNewSession(null);
          await startTwoFactorChallenge(ctx, signedIn.user.id);
          return ctx.json({ twoFactorRedirect: true, twoFactorMethods: ["totp"] });
        }),
      },
    ],
  },
});

export const STEP_UP_PATHS = [
  "/two-factor/enable",
  "/two-factor/get-totp-uri",
  "/two-factor/generate-backup-codes",
  "/two-factor/disable",
  "/link-social",
  "/passkey/generate-register-options",
  "/passkey/verify-registration",
];
export const STEP_UP_SECONDS = 15 * 60;

export function signedInRecently(session, now = Date.now()) {
  const started = new Date(session?.session?.createdAt ?? 0).getTime();
  return Number.isFinite(started) && now - started <= STEP_UP_SECONDS * 1000;
}

export const commyStepUp = () => ({
  id: "commy-step-up",
  hooks: {
    before: [
      {
        matcher: (ctx) => STEP_UP_PATHS.includes(ctx.path),
        handler: createAuthMiddleware(async (ctx) => {
          const session = await getSessionFromCtx(ctx).catch(() => null);
          if (!session) return;
          if (!signedInRecently(session)) {
            throw new APIError("FORBIDDEN", {
              code: "SIGN_IN_AGAIN",
              message: "For your safety, sign in again before changing how this account signs in.",
            });
          }
        }),
      },
    ],
  },
});

export const commyUnlinkLogins = ({ emailSignIn }) => ({
  id: "commy-unlink-logins",
  hooks: {
    before: [
      {
        matcher: (ctx) => ctx.path === "/unlink-account",
        handler: createAuthMiddleware(async (ctx) => {
          const session = await getSessionFromCtx(ctx).catch(() => null);
          if (!session) return;
          const accounts = await ctx.context.internalAdapter.findAccounts(session.user.id);
          if (accounts.length !== 1) return;
          const [only] = accounts;
          if (emailSignIn && only.providerId !== "credential") return;
          throw new APIError("BAD_REQUEST", {
            code: "FAILED_TO_UNLINK_LAST_ACCOUNT",
            message: "This is your only way to sign in. Add a password or another login before removing it.",
          });
        }),
      },
    ],
  },
});

export const commyPasswords = () => ({
  id: "commy-passwords",
  endpoints: {
    setFirstPassword: createAuthEndpoint(
      "/commy/password/set",
      {
        method: "POST",
        use: [freshSessionMiddleware],
        body: z.object({ newPassword: z.string().min(MIN_PASSWORD).max(MAX_PASSWORD) }),
      },
      async (ctx) => {
        const userId = ctx.context.session.user.id;
        const internal = ctx.context.internalAdapter;
        const existing = await internal.findCredentialAccount(userId);
        if (existing?.password) {
          throw new APIError("BAD_REQUEST", {
            code: "PASSWORD_ALREADY_SET",
            message: "This account already has a password. Use Change password instead.",
          });
        }
        const hash = await ctx.context.password.hash(ctx.body.newPassword);
        if (existing) await internal.updateAccount(existing.id, { password: hash });
        else await internal.linkAccount({ userId, providerId: "credential", accountId: userId, password: hash });
        return ctx.json({ status: true });
      },
    ),
  },
});

const NOTICES = {
  "/change-password": {
    subject: "Your password was changed",
    body: "The password for this account was just changed.",
  },
  "/commy/password/set": {
    subject: "A password was added to your account",
    body: "This account can now sign in with an email and password.",
  },
  "/unlink-account": {
    subject: "A sign-in method was removed",
    body: "A Google, Twitch or Apple sign-in was just unlinked from this account.",
  },
  "/two-factor/get-totp-uri": {
    subject: "Your authenticator setup was viewed",
    body: "The setup code for this account's authenticator app was just shown.",
  },
  "/two-factor/generate-backup-codes": {
    subject: "New backup codes were made",
    body: "New two-step sign-in backup codes were made for this account and the old ones stopped working.",
  },
  "/two-factor/disable": {
    subject: "Two-step sign-in was turned off",
    body: "This account no longer asks for an authenticator code when signing in.",
  },
  "/passkey/verify-registration": {
    subject: "A passkey was added to your account",
    body: "A new passkey can now sign in to this account.",
  },
};

const SOCIAL_NAMES = { google: "Google", twitch: "Twitch", apple: "Apple" };

function noticeMail(baseURL, to, subject, body) {
  return {
    to,
    subject,
    heading: subject,
    body: `${body} If this was not you, reset your password and review your account now.`,
    actionLabel: "Review your account",
    url: `${baseURL}/account`,
  };
}

function warnMailFailed(err) {
  console.warn(`[mail] security notice failed: ${err?.code ?? err?.message ?? err}`);
}

export function noticeWhenALoginIsLinked({ baseURL, send }) {
  return async (account, ctx) => {
    const provider = SOCIAL_NAMES[account.providerId];
    if (!provider || !ctx) return;
    const internal = ctx.context.internalAdapter;
    const accounts = await internal.findAccounts(account.userId);
    if (accounts.length < 2) return;
    const user = await internal.findUserById(account.userId);
    if (!user?.email) return;
    await send(
      noticeMail(baseURL, user.email, `A ${provider} sign-in was linked`, `This account can now sign in with ${provider}.`),
    ).catch(warnMailFailed);
  };
}

function succeeded(returned) {
  return returned && !(returned instanceof Error) && !(returned instanceof Response && !returned.ok);
}

export const commySecurityNotices = ({ baseURL, send }) => ({
  id: "commy-security-notices",
  hooks: {
    after: [
      {
        matcher: (ctx) => Object.hasOwn(NOTICES, ctx.path),
        handler: createAuthMiddleware(async (ctx) => {
          const user = ctx.context.session?.user;
          if (!user?.email || !succeeded(ctx.context.returned)) return;
          const notice = NOTICES[ctx.path];
          await send(noticeMail(baseURL, user.email, notice.subject, notice.body)).catch(warnMailFailed);
        }),
      },
    ],
  },
});
