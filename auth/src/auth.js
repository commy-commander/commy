import { betterAuth } from "better-auth";
import { bearer, captcha, deviceAuthorization, magicLink } from "better-auth/plugins";
import pg from "pg";
import { commyDeviceRequests } from "./device-requests.js";
import { commyDevices } from "./devices.js";
import { commyInvites } from "./invites.js";
import { createMailer } from "./mailer.js";

export function buildAuthOptions(config, { pool, send }) {
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
    commyInvites({ serverUrl: config.baseURL, requireAccount: config.invitesRequireAccount }),
  ];

  if (config.turnstile) {
    plugins.push(
      captcha({
        provider: "cloudflare-turnstile",
        secretKey: config.turnstile.secretKey,
        endpoints: ["/sign-up/email", "/sign-in/email", "/sign-in/magic-link", "/request-password-reset"],
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
      sendResetPassword: ({ user, url }) =>
        send({
          to: user.email,
          subject: `Reset your ${config.instanceName} password`,
          heading: "Reset your password",
          body: "Someone asked to reset the password for this account.",
          actionLabel: "Choose a new password",
          url,
        }),
    },
    emailVerification: {
      sendOnSignUp: Boolean(config.smtp),
      autoSignInAfterVerification: true,
      sendVerificationEmail: ({ user, url }) =>
        send({
          to: user.email,
          subject: `Verify your ${config.instanceName} email`,
          heading: "Verify your email",
          body: "Confirm this address to finish creating your account.",
          actionLabel: "Verify email",
          url,
        }),
    },
    socialProviders: config.google
      ? { google: { clientId: config.google.clientId, clientSecret: config.google.clientSecret, disableSignUp: !config.allowSignup } }
      : {},
    account: {
      accountLinking: { enabled: true, trustedProviders: ["google"] },
      // Google tokens are never used after sign-in, so keep them unreadable at rest
      encryptOAuthTokens: true,
    },
    databaseHooks: {
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
      customRules: {
        "/sign-up/email": { window: 600, max: 5 },
        "/sign-in/email": { window: 60, max: 10 },
        "/sign-in/magic-link": { window: 600, max: 5 },
        "/request-password-reset": { window: 600, max: 5 },
        "/device/code": { window: 60, max: 10 },
        "/device/approve": { window: 60, max: 10 },
        "/commy/devices/challenge": { window: 60, max: 20 },
        "/commy/invites/create": { window: 3600, max: 30 },
        "/commy/invites/resolve": { window: 60, max: 60 },
      },
    },
    telemetry: { enabled: false },
    advanced: {
      useSecureCookies: config.baseURL.startsWith("https://"),
    },
    plugins,
  };
}

export function createAuth(config) {
  const pool = new pg.Pool({ connectionString: config.databaseUrl, max: 10 });
  const send = createMailer(config);
  const options = buildAuthOptions(config, { pool, send });
  return { auth: betterAuth(options), options, pool };
}
