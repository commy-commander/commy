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

function oauthClient(clientId, clientSecret) {
  return clientId && clientSecret ? { clientId, clientSecret } : null;
}

// Mail settings come either as one SMTP_URL or as separate SMTP_HOST,
// SMTP_PORT, SMTP_USER and SMTP_PASSWORD values. The separate form avoids
// URL-encoding passwords and app tokens.
function smtpConfig(env, errors) {
  const host = (env.SMTP_HOST ?? "").trim();
  if (!env.SMTP_URL && !host) return null;

  let transport;
  let user = "";
  if (env.SMTP_URL) {
    transport = env.SMTP_URL;
    try {
      user = decodeURIComponent(new URL(env.SMTP_URL).username);
    } catch {
      errors.push("SMTP_URL must look like smtp://user:password@host:587 or smtps://...:465");
    }
  } else {
    const port = Number(env.SMTP_PORT || 587);
    if (!Number.isInteger(port) || port <= 0 || port > 65535) {
      errors.push("SMTP_PORT must be a valid TCP port");
    }
    user = (env.SMTP_USER ?? "").trim();
    const pass = env.SMTP_PASSWORD ?? "";
    if (user && !pass) errors.push("SMTP_PASSWORD is required when SMTP_USER is set");
    // 465 is TLS from the first byte; anything else must upgrade with STARTTLS
    const secure = env.SMTP_SECURE ? env.SMTP_SECURE === "true" : port === 465;
    transport = {
      host,
      port,
      secure,
      requireTLS: !secure && env.SMTP_REQUIRE_TLS !== "false",
      ...(user ? { auth: { user, pass } } : {}),
    };
  }

  const fallbackFrom = user.includes("@") ? `Commy <${user}>` : "Commy <no-reply@localhost>";
  return { transport, from: env.EMAIL_FROM || fallbackFrom };
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

  const smtp = smtpConfig(env, errors);

  if (errors.length > 0) {
    throw new Error(`Invalid auth server configuration:\n- ${errors.join("\n- ")}`);
  }

  const google = oauthClient(env.GOOGLE_CLIENT_ID, env.GOOGLE_CLIENT_SECRET);
  const twitch = oauthClient(env.TWITCH_CLIENT_ID, env.TWITCH_CLIENT_SECRET);

  const turnstile =
    env.TURNSTILE_SITE_KEY && env.TURNSTILE_SECRET_KEY
      ? { siteKey: env.TURNSTILE_SITE_KEY, secretKey: env.TURNSTILE_SECRET_KEY }
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
    twitch,
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
      twitch: Boolean(config.twitch),
      magicLink: Boolean(config.smtp),
      emailVerification: Boolean(config.smtp),
      passwordReset: Boolean(config.smtp),
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
