import { normalizeAccessCode } from "./access.js";
import { PRIVATE_NETWORKS, trustedProxyList } from "./client-ip.js";
import { LINK_BLOCKLIST_PATH, linkBlocklistSources } from "./link-blocklist.js";
import { appleAppSiteAssociation, assetLinks } from "./app-sign-in.js";

export const APPLE_ORIGIN = "https://appleid.apple.com";
export const CAPTCHA_PATH = "/commy/captcha";
const DEFAULT_PORT = 3001;
const SMTP_SUBMISSION_PORT = 587;
const SMTPS_IMPLICIT_TLS_PORT = 465;

function list(value) {
  return (value ?? "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

function validPort(n) {
  return Number.isInteger(n) && n > 0 && n <= 65535;
}

function stripSlash(url) {
  return url.replace(/\/+$/, "");
}

function oauthClient(env, prefix, warnings, uses) {
  const clientId = (env[`${prefix}_CLIENT_ID`] ?? "").trim();
  const clientSecret = (env[`${prefix}_CLIENT_SECRET`] ?? "").trim();
  if (clientId && clientSecret) return { clientId, clientSecret };
  if (clientId || clientSecret) {
    const missing = clientId ? `${prefix}_CLIENT_SECRET` : `${prefix}_CLIENT_ID`;
    warnings.push(`${missing} is empty, so ${uses} stays off until both ${prefix}_CLIENT_ID and ${prefix}_CLIENT_SECRET are set`);
  }
  return null;
}

const COMMY_IOS_APP = "chat.commy.mobile";
const COMMY_IOS_APP_ID = `U5SWV4RDXH.${COMMY_IOS_APP}`;
const CERT_FINGERPRINT = /^([0-9A-F]{2}:){31}[0-9A-F]{2}$/;

function phoneApps(env, errors) {
  const ios = (env.IOS_APP_IDS ?? "").trim();
  const iosAppIds = ios === "off" ? [] : ios ? list(ios) : [COMMY_IOS_APP_ID];
  const fingerprints = list(env.ANDROID_APP_CERT_SHA256).map((f) => f.toUpperCase());
  const bad = fingerprints.filter((f) => !CERT_FINGERPRINT.test(f));
  if (bad.length) errors.push("ANDROID_APP_CERT_SHA256 must be SHA-256 certificate fingerprints like AB:CD:...:EF (32 pairs), comma-separated");
  const androidPackage = (env.ANDROID_APP_PACKAGE ?? "").trim() || COMMY_IOS_APP;
  return { iosAppIds, android: fingerprints.length ? { package: androidPackage, fingerprints } : null };
}

function applePrivateKey(value) {
  return value.replace(/\\n/g, "\n").trim();
}

function appleConfig(env, warnings, errors) {
  const bundles = (env.APPLE_APP_BUNDLE_IDS ?? "").trim();
  const appBundleIds = bundles === "off" ? [] : bundles ? list(bundles) : [COMMY_IOS_APP];
  const servicesId = (env.APPLE_CLIENT_ID ?? "").trim();
  const teamId = (env.APPLE_TEAM_ID ?? "").trim();
  const keyId = (env.APPLE_KEY_ID ?? "").trim();
  const privateKey = applePrivateKey(env.APPLE_PRIVATE_KEY ?? "");
  const webParts = { APPLE_CLIENT_ID: servicesId, APPLE_TEAM_ID: teamId, APPLE_KEY_ID: keyId, APPLE_PRIVATE_KEY: privateKey };
  const missing = Object.entries(webParts).filter(([, value]) => !value).map(([name]) => name);
  let web = null;
  if (missing.length === 0) {
    if (!privateKey.includes("PRIVATE KEY")) errors.push("APPLE_PRIVATE_KEY must be the contents of the .p8 key file from Apple");
    else web = { servicesId, teamId, keyId, privateKey };
  } else if (missing.length < 4) {
    warnings.push(`${missing.join(", ")} empty, so Sign in with Apple on the website stays off until all four APPLE_ values are set`);
  }
  if (!web && appBundleIds.length === 0) return null;
  return { appBundleIds, web };
}

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
    const port = Number(env.SMTP_PORT || SMTP_SUBMISSION_PORT);
    if (!validPort(port)) {
      errors.push("SMTP_PORT must be a valid TCP port");
    }
    user = (env.SMTP_USER ?? "").trim();
    const pass = env.SMTP_PASSWORD ?? "";
    if (user && !pass) errors.push("SMTP_PASSWORD is required when SMTP_USER is set");
    const secure = env.SMTP_SECURE ? env.SMTP_SECURE === "true" : port === SMTPS_IMPLICIT_TLS_PORT;
    transport = {
      host,
      port,
      secure,
      requireTLS: !secure && env.SMTP_REQUIRE_TLS !== "false",
      ...(user ? { auth: { user, pass } } : {}),
    };
  }

  const fallbackFrom = user.includes("@") ? `Commy <${user}>` : "Commy <no-reply@localhost>";
  const replyTo = (env.EMAIL_REPLY_TO ?? "").trim() || null;
  return { transport, from: env.EMAIL_FROM || fallbackFrom, replyTo };
}

function trustedProxiesConfig(env, errors) {
  const value = (env.TRUSTED_PROXIES ?? "").trim();
  const ranges = value === "" ? PRIVATE_NETWORKS : value === "none" ? [] : list(value);
  try {
    return trustedProxyList(ranges);
  } catch (err) {
    errors.push(`TRUSTED_PROXIES must be "none" or comma-separated IP addresses or CIDR ranges: ${err.message}`);
    return null;
  }
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
  if (!validPort(port)) {
    errors.push("PORT must be a valid TCP port");
  }

  const smtp = smtpConfig(env, errors);
  const phones = phoneApps(env, errors);
  const appleWarnings = [];
  const apple = appleConfig(env, appleWarnings, errors);
  const trustedProxies = trustedProxiesConfig(env, errors);

  if (errors.length > 0) {
    throw new Error(`Invalid auth server configuration:\n- ${errors.join("\n- ")}`);
  }

  const warnings = [...appleWarnings];
  const google = oauthClient(env, "GOOGLE", warnings, "Google sign-in");
  const twitch = oauthClient(env, "TWITCH", warnings, "Twitch sign-in");
  const igdb = oauthClient(env, "IGDB", warnings, "the separate IGDB app") ?? twitch;
  const klipyKey = (env.KLIPY_API_KEY ?? "").trim() || null;

  const syncUrl = (env.SYNC_URL ?? "").trim().replace(/\/+$/, "") || null;
  if (syncUrl && !/^wss?:\/\/[^/\s]+\/\S*$/.test(`${syncUrl}/`)) {
    throw new Error("Invalid auth server configuration:\n- SYNC_URL must be the sync service's WebSocket address, for example wss://commy.chat/sync");
  }

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
    trustedOrigins: [...new Set([baseURL, ...(apple?.web ? [APPLE_ORIGIN] : []), ...list(env.TRUSTED_ORIGINS)])],
    instanceName: env.COMMY_INSTANCE_NAME || "Commy",
    postalAddress: (env.EMAIL_POSTAL_ADDRESS ?? "").trim() || null,
    desktopClientIds: list(env.DEVICE_CLIENT_IDS).length > 0 ? list(env.DEVICE_CLIENT_IDS) : ["commy-desktop"],
    allowSignup: env.ALLOW_SIGNUP !== "false",
    accessCode: normalizeAccessCode(env.ACCESS_CODE) || null,
    invitesRequireAccount: env.INVITES_REQUIRE_ACCOUNT === "true",
    invitesMaxPermanent: Number(env.INVITES_MAX_PERMANENT) > 0 ? Math.floor(Number(env.INVITES_MAX_PERMANENT)) : undefined,
    doorbell: env.DOORBELL !== "false",
    syncUrl,
    linkBlocklistSources: linkBlocklistSources(env.LINK_BLOCKLIST_URLS),
    redisUrl: (env.REDIS_URL ?? "").trim() || null,
    trustedProxies,
    google,
    twitch,
    apple,
    phones,
    smtp,
    turnstile,
    igdb,
    klipyKey,
    warnings,
  };
}

export function wellKnownFiles(config) {
  const files = {};
  if (config.phones.iosAppIds.length) {
    files["/.well-known/apple-app-site-association"] = JSON.stringify(appleAppSiteAssociation(config.phones.iosAppIds));
  }
  if (config.phones.android) files["/.well-known/assetlinks.json"] = JSON.stringify(assetLinks(config.phones.android));
  return files;
}

export function featureSummary(config) {
  const on = (value) => (value ? "on" : "off");
  return [
    `google=${on(config.google)}`,
    `twitch=${on(config.twitch)}`,
    `apple=${config.apple?.web ? "app+web" : on(config.apple)}`,
    `email=${on(config.smtp)}`,
    `turnstile=${on(config.turnstile)}`,
    `gifs=${on(config.klipyKey)}`,
    `games=${on(config.igdb)}${config.igdb && config.igdb === config.twitch ? " (twitch app)" : ""}`,
    `doorbell=${on(config.doorbell)}`,
    `sync=${on(config.syncUrl)}`,
    `link_blocklist=${on(config.linkBlocklistSources.length)}`,
    `redis=${on(config.redisUrl)}`,
    `signup=${on(config.allowSignup)}`,
    `access_code=${on(config.accessCode)}`,
  ].join(" ");
}

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
      apple: Boolean(config.apple?.web),
      appleApp: Boolean(config.apple?.appBundleIds.length),
      magicLink: Boolean(config.smtp),
      emailVerification: Boolean(config.smtp),
      passwordReset: Boolean(config.smtp),
      emailPassword: true,
      passkey: true,
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
      signInEmail: `${api}/sign-in/email`,
      signInSocial: `${api}/sign-in/social`,
      appSignInStart: `${api}/commy/app-sign-in/start`,
      appSignInFinish: `${api}/commy/app-sign-in/finish`,
      passkeyOptions: `${api}/passkey/generate-authenticate-options`,
      passkeyVerify: `${api}/passkey/verify-authentication`,
      twoFactorVerify: `${api}/two-factor/verify-totp`,
      twoFactorBackupCode: `${api}/two-factor/verify-backup-code`,
      ...(config.turnstile ? { captcha: `${api}${CAPTCHA_PATH}` } : {}),
      ...(config.accessCode ? { accessCode: `${api}/commy/access` } : {}),
      signOut: `${api}/sign-out`,
      devices: `${api}/commy/devices`,
      linkChallenge: `${api}/commy/devices/challenge`,
      registerDevice: `${api}/commy/devices/register`,
      removeDevice: `${api}/commy/devices/remove`,
      accountDeleteStart: `${api}/commy/account/delete/start`,
      accountDeleteFinish: `${api}/commy/account/delete/finish`,
      createInvite: `${api}/commy/invites/create`,
      resolveInvite: `${api}/commy/invites/resolve`,
      directoryList: `${api}/commy/directory/list`,
      directoryPublish: `${api}/commy/directory/publish`,
      directoryUnlist: `${api}/commy/directory/unlist`,
      gifSearch: `${api}/commy/gifs/search`,
      stickerSearch: `${api}/commy/stickers/search`,
      gameSearch: `${api}/commy/games/search`,
      emojiSearch: `${api}/commy/emojis/search`,
      media: `${api}/commy/media`,
      vault: `${api}/commy/vault`,
      vaultPut: `${api}/commy/vault/put`,
      folderKey: `${api}/commy/folder-key`,
      doorbellRing: `${api}/commy/doorbell/ring`,
      doorbellTake: `${api}/commy/doorbell/take`,
      sync: config.syncUrl,
      ...(config.linkBlocklistSources.length ? { linkBlocklist: `${api}${LINK_BLOCKLIST_PATH}` } : {}),
    },
    features: {
      gifs: Boolean(config.klipyKey),
      stickers: Boolean(config.klipyKey),
      games: Boolean(config.igdb),
      emojis: true,
      media: true,
      vault: true,
      folderLock: true,
      doorbell: Boolean(config.doorbell),
      twoFactor: true,
      accessCode: Boolean(config.accessCode),
      changeEmail: Boolean(config.smtp),
      sync: Boolean(config.syncUrl),
      linkBlocklist: config.linkBlocklistSources.length > 0,
    },
    invites: {
      linkPrefix: `${config.baseURL}/i/`,
      requireAccount: config.invitesRequireAccount,
    },
  };
}
