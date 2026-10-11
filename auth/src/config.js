// Turns environment variables into a validated config object. Kept free of
// side effects so it can be unit tested without a database.

const DEFAULT_PORT = 3001;

function list(value) {
  return (value ?? "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

function stripSlash(url) {
  return url.replace(/\/+$/, "");
}

export function loadConfig(env = process.env) {
  const errors = [];

  const secret = env.BETTER_AUTH_SECRET ?? "";
  if (secret.length < 32) {
    errors.push("BETTER_AUTH_SECRET must be at least 32 characters (openssl rand -base64 32)");
  }

  const databaseUrl = env.DATABASE_URL ?? "";
  if (!databaseUrl.startsWith("postgres")) {
    errors.push("DATABASE_URL must be a postgres:// connection string");
  }

  let baseURL = env.BETTER_AUTH_URL ?? "";
  try {
    baseURL = stripSlash(new URL(baseURL).toString());
  } catch {
    errors.push("BETTER_AUTH_URL must be the public URL of the site, for example https://commy.chat");
  }

  const port = Number(env.PORT ?? DEFAULT_PORT);
  if (!Number.isInteger(port) || port <= 0 || port > 65535) {
    errors.push("PORT must be a valid TCP port");
  }

  if (errors.length > 0) {
    throw new Error(`Invalid auth server configuration:\n- ${errors.join("\n- ")}`);
  }

  const google =
    env.GOOGLE_CLIENT_ID && env.GOOGLE_CLIENT_SECRET
      ? { clientId: env.GOOGLE_CLIENT_ID, clientSecret: env.GOOGLE_CLIENT_SECRET }
      : null;

  const turnstile =
    env.TURNSTILE_SITE_KEY && env.TURNSTILE_SECRET_KEY
      ? { siteKey: env.TURNSTILE_SITE_KEY, secretKey: env.TURNSTILE_SECRET_KEY }
      : null;

  const smtp = env.SMTP_URL
    ? { url: env.SMTP_URL, from: env.EMAIL_FROM || "Commy <no-reply@localhost>" }
    : null;

  return {
    port,
    secret,
    databaseUrl,
    baseURL,
    basePath: "/api/auth",
    trustedOrigins: [...new Set([baseURL, ...list(env.TRUSTED_ORIGINS)])],
    instanceName: env.COMMY_INSTANCE_NAME || "Commy",
    desktopClientIds: list(env.DEVICE_CLIENT_IDS).length > 0 ? list(env.DEVICE_CLIENT_IDS) : ["commy-desktop"],
    allowSignup: env.ALLOW_SIGNUP !== "false",
    invitesRequireAccount: env.INVITES_REQUIRE_ACCOUNT === "true",
    google,
    smtp,
    turnstile,
  };
}

// Public discovery document the desktop app reads from a custom server URL.
export function discoveryDocument(config) {
  const api = `${config.baseURL}${config.basePath}`;
  return {
    name: config.instanceName,
    version: 1,
    issuer: config.baseURL,
    api,
    providers: {
      google: Boolean(config.google),
      magicLink: Boolean(config.smtp),
      emailPassword: true,
    },
    turnstileSiteKey: config.turnstile?.siteKey ?? null,
    deviceAuthorization: {
      clientId: config.desktopClientIds[0],
      deviceCodeEndpoint: `${api}/device/code`,
      tokenEndpoint: `${api}/device/token`,
      verificationUri: `${config.baseURL}/device`,
    },
    endpoints: {
      session: `${api}/get-session`,
      signOut: `${api}/sign-out`,
      devices: `${api}/commy/devices`,
      linkChallenge: `${api}/commy/devices/challenge`,
      registerDevice: `${api}/commy/devices/register`,
      removeDevice: `${api}/commy/devices/remove`,
      createInvite: `${api}/commy/invites/create`,
      resolveInvite: `${api}/commy/invites/resolve`,
    },
    invites: {
      linkPrefix: `${config.baseURL}/i/`,
      requireAccount: config.invitesRequireAccount,
    },
  };
}
