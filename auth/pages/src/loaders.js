import { APPLY_PAGE, cleanToken, REVIEW_PAGE } from "./lib/listing.js";
import { safeNext } from "./lib/safe-next.js";
import { cleanCode, cleanInviteCode } from "./lib/user-code.js";

const ACCOUNT_ERRORS = {
  token_expired: "That link has expired. Sign in to get a new one.",
  invalid_token: "That link is not valid any more. Sign in to get a new one.",
  account_not_linked: "That sign-in could not be linked to an existing account. Sign in with your email first.",
  signup_disabled: "This server is not accepting new accounts.",
  email_not_found: "The provider did not share an email address, which a Commy account needs.",
  access_denied: "Sign-in was cancelled.",
  access_code_required: "New accounts need the access code first. Choose Create an account below to enter it.",
  adult_confirmation_required: "New accounts need you to confirm you are 18 or older. Choose Create an account below.",
  account_already_linked_to_different_user: "That login already belongs to another Commy account.",
  unable_to_link_account: "That login could not be linked. Its email address needs to be verified with the provider first.",
};

const ACCOUNT_NOTICES = {
  email: "Email confirmed. If you just confirmed from your old address, open the last link we sent to the new one to finish.",
  password: "Password saved. You can now sign in with your email and password.",
  linked: "Login linked. You can now use it to sign in.",
  unlinked: "Login unlinked.",
  passkey: "Passkey added. Choose Passkey when you sign in to use it.",
  "two-factor-on": "Two-step sign-in is on.",
  "two-factor-off": "Two-step sign-in is off.",
  "backup-codes": "New backup codes saved. The old ones no longer work.",
  device: "Device unlinked. That app is signed out the next time it connects.",
  everywhere: "Every other browser and desktop app is signed out.",
};

const DELETE_NOTICES = {
  deleted: "Your account was deleted. We sent one last email to confirm it.",
};

const SIGN_IN_DID_NOT_COMPLETE = "Sign-in did not complete, please try again.";
const INVITE_PREFIX = "/i/";
const MAX_RESET_TOKEN = 256;

export function accountError(code) {
  const key = String(code ?? "").toLowerCase();
  if (!key) return "";
  return Object.hasOwn(ACCOUNT_ERRORS, key) ? ACCOUNT_ERRORS[key] : SIGN_IN_DID_NOT_COMPLETE;
}

function known(table, key) {
  return Object.hasOwn(table, key ?? "") ? table[key] : "";
}

export function providersFrom(discovery, { adultsOnly = false } = {}) {
  const providers = discovery?.providers ?? {};
  const features = discovery?.features ?? {};
  return {
    google: providers.google ?? false,
    twitch: providers.twitch ?? false,
    magicLink: providers.magicLink ?? false,
    passwordReset: providers.passwordReset ?? false,
    emailPassword: providers.emailPassword ?? true,
    passkey: providers.passkey ?? false,
    changeEmail: features.changeEmail ?? false,
    twoFactor: features.twoFactor ?? false,
    adultsOnly,
    turnstileSiteKey: discovery?.turnstileSiteKey ?? null,
  };
}

function pick(list, keys) {
  return (list ?? []).map((item) => Object.fromEntries(keys.map((key) => [key, item[key] ?? null])));
}

export function createLoaders(server) {
  const { fetchAuth, name, providers, hasAccess, listing = false } = server;

  const sessionUser = async () => {
    const data = await fetchAuth("/get-session");
    if (!data?.user) return null;
    const { id, name: userName, email, image, twoFactorEnabled } = data.user;
    return { id, name: userName, email, image: image ?? null, twoFactorEnabled: Boolean(twoFactorEnabled) };
  };

  const devices = async () => pick((await fetchAuth("/commy/devices"))?.devices, ["id", "nodeId", "name", "createdAt", "lastSeenAt"]);
  const accounts = async () => pick(await fetchAuth("/list-accounts"), ["id", "providerId"]);
  const passkeys = async () => pick(await fetchAuth("/passkey/list-user-passkeys"), ["id", "name", "createdAt"]);

  async function account(query) {
    const user = await sessionUser();
    const next = safeNext(query.get("next"));
    if (user && next !== "/account") return { redirect: next };
    const errorMessage = accountError(query.get("error"));
    const page = { page: "account", title: `Account - ${name}` };
    if (!user) return { ...page, props: { user: null, providers, next, errorMessage } };
    const [linked, ways, keys] = await Promise.all([devices(), accounts(), passkeys()]);
    const notice = known(ACCOUNT_NOTICES, query.get("notice"));
    return { ...page, props: { user, providers, next, errorMessage, notice, devices: linked, accounts: ways, passkeys: keys } };
  }

  async function signUp(query, cookie) {
    const next = safeNext(query.get("next"));
    if (await sessionUser()) return { redirect: next };
    const locked = !hasAccess(cookie);
    return { page: "sign-up", title: `Create a ${name} account`, props: { locked, providers: locked ? null : providers, next } };
  }

  async function twoFactor(query) {
    const next = safeNext(query.get("next"));
    if (await sessionUser()) return { redirect: next };
    return { page: "two-factor", title: `Two-step sign-in - ${name}`, props: { next } };
  }

  function resetPassword(query) {
    const token = query.get("error") ? "" : (query.get("token") ?? "").slice(0, MAX_RESET_TOKEN);
    return { page: "reset-password", title: `Reset password - ${name}`, props: { token } };
  }

  function verifyEmail(query) {
    return {
      page: "verify-email",
      title: `Confirm your email - ${name}`,
      robots: "noindex",
      props: { result: query.get("result") ?? "", next: safeNext(query.get("next")) },
    };
  }

  async function deleteAccount(query) {
    const notice = known(DELETE_NOTICES, query.get("notice"));
    const user = notice ? null : await sessionUser();
    return {
      page: "delete-account",
      title: `Delete your account - ${name}`,
      description: `How to delete your ${name} account, in the app or on this page, and what is deleted.`,
      props: { notice, email: user?.email ?? null, providers: user || notice ? null : providers },
    };
  }

  async function device(query) {
    const code = cleanCode(query.get("user_code") ?? "");
    const user = await sessionUser();
    return {
      page: "device",
      title: `Sign in a device - ${name}`,
      props: {
        code,
        email: user?.email ?? null,
        providers: user ? null : providers,
        returnTo: code ? `/device?user_code=${code}` : "/device",
      },
    };
  }

  async function resolveInvite(code) {
    if (!code) return null;
    const data = await fetchAuth(`/commy/invites/resolve?code=${encodeURIComponent(code)}`);
    if (!data) return null;
    return {
      code: data.code,
      url: data.url,
      serverName: data.sealed ? null : (data.serverName ?? null),
      sealed: data.sealed && data.ticket ? data.ticket : null,
      expiresAt: data.expiresAt ?? null,
      hasBackground: Boolean(data.sealed && data.hasBackground),
    };
  }

  async function invite(segment) {
    const resolved = await resolveInvite(cleanInviteCode(segment));
    if (!resolved) return { page: "invite", status: 404, title: `Invite not found - ${name}`, robots: "noindex", props: { invite: null } };
    const title = resolved.serverName ? `Join ${resolved.serverName} on ${name}` : `Join a server on ${name}`;
    return {
      page: "invite",
      title,
      description: "You have been invited to a Commy server. Commy is free, open source, peer to peer chat and voice.",
      openGraph: { title, siteName: name },
      robots: "noindex",
      props: { invite: resolved },
    };
  }

  function listServer(query) {
    if (!listing) return null;
    return {
      page: "list-server",
      title: `List your server - ${name}`,
      description: `How community servers get on ${name}'s list, the pledge their operators sign, and the checks they pass every month.`,
      robots: query.get("confirm") ? "noindex" : null,
      props: { confirm: cleanToken(query.get("confirm"), 200) || null },
    };
  }

  async function listingReview(query) {
    if (!listing) return null;
    const token = cleanToken(query.get("token"));
    const data = token ? await fetchAuth(`/commy/listing/review?token=${encodeURIComponent(token)}`) : null;
    return { page: "listing-review", title: `Review a listing - ${name}`, robots: "noindex", props: { token, application: data?.application ?? null } };
  }

  const routes = {
    [APPLY_PAGE]: listServer,
    [REVIEW_PAGE]: listingReview,
    "/account": account,
    "/signup": signUp,
    "/account/two-factor": twoFactor,
    "/account/reset-password": resetPassword,
    "/account/verify-email": verifyEmail,
    "/delete-account": deleteAccount,
    "/device": device,
  };

  return function load(pathname, query, cookie) {
    if (Object.hasOwn(routes, pathname)) return routes[pathname](query, cookie);
    const segment = inviteSegment(pathname);
    return segment === null ? null : invite(segment);
  };
}

export const PAGE_PATHS = [APPLY_PAGE, REVIEW_PAGE, "/account", "/signup", "/account/two-factor", "/account/reset-password", "/account/verify-email", "/delete-account", "/device"];

function inviteSegment(pathname) {
  if (!pathname.startsWith(INVITE_PREFIX)) return null;
  const segment = pathname.slice(INVITE_PREFIX.length);
  return segment && !segment.includes("/") ? segment : null;
}

export function isPagePath(pathname) {
  return PAGE_PATHS.includes(pathname) || inviteSegment(pathname) !== null;
}
