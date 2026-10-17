import { betterAuth } from "better-auth";
import { bearer, captcha, deviceAuthorization, magicLink } from "better-auth/plugins";
import { APIError } from "better-auth/api";
import pg from "pg";
import { hasAccess } from "./access.js";
import { commyDeviceRequests } from "./device-requests.js";
import { commyDevices } from "./devices.js";
import { commyDoorbell } from "./doorbell.js";
import { commyInvites } from "./invites.js";
import { commyMedia } from "./media.js";
import { commyVault } from "./vault.js";
import { createMailer } from "./mailer.js";
import { redisRateLimitStorage } from "./rate-limit.js";

export function socialProviders(config) {
  const providers = {};
  for (const name of ["google", "twitch"]) {
    const client = config[name];
    if (client) providers[name] = { clientId: client.clientId, clientSecret: client.clientSecret, disableSignUp: !config.allowSignup };
  }
  return providers;
}

// `redis` is a connected handle from connectRedis, or null to keep doorbell
// rings in the database and rate-limit counters in memory
export function buildAuthOptions(config, { pool, send, fetch, redis = null }) {
  const plugins = [
    // Desktop clients hold a session token and send it as a Bearer header
    bearer(),
    deviceAuthorization({
      expiresIn: "10m",
      interval: "5s",
      verificationUri: `${config.baseURL}/device`,
      validateClient: (clientId) => config.desktopClientIds.includes(clientId),
    }),
    commyDevices({ secret: config.secret, serverUrl: config.baseURL }),
    commyDeviceRequests({ secret: config.secret }),
    commyInvites({
      serverUrl: config.baseURL,
      requireAccount: config.invitesRequireAccount,
      maxPermanent: config.invitesMaxPermanent,
    }),
    commyMedia({ secret: config.secret, klipyKey: config.klipyKey, igdb: config.igdb, fetch }),
    commyVault(),
  ];

  // Content-free wake-ups for messages sent while their recipient was offline
  if (config.doorbell) plugins.push(commyDoorbell({ redis }));

  if (config.turnstile) {
    plugins.push(
      captcha({
        provider: "cloudflare-turnstile",
        secretKey: config.turnstile.secretKey,
        endpoints: [
          "/sign-up/email",
          "/sign-in/email",
          "/sign-in/magic-link",
          "/request-password-reset",
          "/send-verification-email",
        ],
        allowedHostnames: [new URL(config.baseURL).hostname],
      }),
    );
  }

  if (config.smtp) {
    plugins.push(
      magicLink({
        disableSignUp: !config.allowSignup,
        sendMagicLink: ({ email, url }) =>
          send({
            to: email,
            subject: `Sign in to ${config.instanceName}`,
            heading: `Sign in to ${config.instanceName}`,
            body: "Use the button below to finish signing in. The link expires in 5 minutes.",
            actionLabel: "Sign in",
            url,
          }),
      }),
    );
  }

  return {
    appName: config.instanceName,
    baseURL: config.baseURL,
    basePath: config.basePath,
    secret: config.secret,
    trustedOrigins: config.trustedOrigins,
    database: pool,
    emailAndPassword: {
      enabled: true,
      disableSignUp: !config.allowSignup,
      // Verification needs mail; without SMTP accounts work unverified
      requireEmailVerification: Boolean(config.smtp),
      resetPasswordTokenExpiresIn: 3600,
      // A reset is how people recover a stolen account, so it ends every other session
      revokeSessionsOnPasswordReset: true,
      sendResetPassword: ({ user, url }) =>
        send({
          to: user.email,
          subject: `Reset your ${config.instanceName} password`,
          heading: "Reset your password",
          body: "Someone asked to reset the password for this account. The link expires in 1 hour and signs out every other device.",
          actionLabel: "Choose a new password",
          url,
        }),
    },
    emailVerification: {
      sendOnSignUp: Boolean(config.smtp),
      // Signing in with an unverified address sends a fresh link
      sendOnSignIn: Boolean(config.smtp),
      autoSignInAfterVerification: true,
      expiresIn: 24 * 3600,
      sendVerificationEmail: ({ user, url }) =>
        send({
          to: user.email,
          subject: `Verify your ${config.instanceName} email`,
          heading: "Verify your email",
          body: "Confirm this address to finish creating your account. The link expires in 24 hours.",
          actionLabel: "Verify email",
          url,
        }),
    },
    socialProviders: socialProviders(config),
    account: {
      // Google only issues verified addresses. Twitch links only when it reports
      // the address as verified, which Better Auth checks for untrusted providers.
      accountLinking: { enabled: true, trustedProviders: ["google"] },
      // Provider tokens are never used after sign-in, so keep them unreadable at rest
      encryptOAuthTokens: true,
    },
    databaseHooks: {
      user: {
        create: {
          before: async (user, ctx) => {
            if (!config.accessCode) return;
            const cookie = ctx?.headers?.get("cookie") ?? ctx?.request?.headers.get("cookie");
            if (!hasAccess(cookie, config.accessCode)) {
              throw new APIError("FORBIDDEN", {
                code: "ACCESS_CODE_REQUIRED",
                message: "Creating an account needs the access code. Enter it on the sign-up page first.",
              });
            }
          },
        },
      },
      session: {
        create: {
          // IPs are still used for rate limiting in memory, but never stored
          before: async (session) => ({ data: { ...session, ipAddress: null, userAgent: null } }),
        },
      },
    },
    rateLimit: {
      enabled: true,
      window: 60,
      max: 100,
      // Shared, restart-proof counters under keyed hashes; in memory without Redis
      ...(redis ? { customStorage: redisRateLimitStorage(redis, config.secret) } : {}),
      customRules: {
        "/sign-up/email": { window: 600, max: 5 },
        "/sign-in/email": { window: 60, max: 10 },
        "/sign-in/magic-link": { window: 600, max: 5 },
        "/request-password-reset": { window: 600, max: 5 },
        "/send-verification-email": { window: 600, max: 5 },
        "/sign-in/social": { window: 60, max: 20 },
        "/device/code": { window: 60, max: 10 },
        "/device/approve": { window: 60, max: 10 },
        "/commy/devices/challenge": { window: 60, max: 20 },
        "/commy/invites/create": { window: 3600, max: 30 },
        "/commy/invites/resolve": { window: 60, max: 60 },
        "/commy/gifs/search": { window: 60, max: 60 },
        "/commy/games/search": { window: 60, max: 60 },
        "/commy/doorbell/ring": { window: 60, max: 300 },
        "/commy/doorbell/take": { window: 60, max: 600 },
      },
    },
    telemetry: { enabled: false },
    advanced: {
      useSecureCookies: config.baseURL.startsWith("https://"),
    },
    plugins,
  };
}

export function createAuth(config, { redis = null } = {}) {
  const pool = new pg.Pool({ connectionString: config.databaseUrl, max: 10 });
  const send = createMailer(config);
  const options = buildAuthOptions(config, { pool, send, redis });
  return { auth: betterAuth(options), options, pool };
}
