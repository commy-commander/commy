const HOME = "/";
const ACCOUNT = "/account";
export const APP_SIGN_UP_NEXT = "/account?app=1";
export const APP_RETURN_LINK = "commy://email-confirmed";
const OPEN_APP = { label: "Open Commy", href: APP_RETURN_LINK };

function readResult(value) {
  return value === "confirmed" || value === "expired" ? value : "invalid";
}

function signInHref(next) {
  return next === ACCOUNT ? ACCOUNT : `${ACCOUNT}?next=${encodeURIComponent(next)}`;
}

function appOutcome(result, serverName) {
  if (result === "confirmed") {
    return {
      result,
      title: "Email confirmed",
      body: `Thanks, your email address is confirmed and your ${serverName} account is ready. Open Commy to finish signing in.`,
      action: OPEN_APP,
    };
  }
  return {
    result,
    title: result === "expired" ? "This link has expired" : "This link did not work",
    body: "Open Commy and tap Send the email again to get a new link.",
    action: OPEN_APP,
  };
}

export function verifyEmailOutcome(rawResult, safeNext, serverName = "Commy") {
  const result = readResult(rawResult);
  if (safeNext === APP_SIGN_UP_NEXT) return appOutcome(result, serverName);
  const next = safeNext === HOME ? ACCOUNT : safeNext;

  if (result === "confirmed") {
    return {
      result,
      title: "Email confirmed",
      body: `Thanks, your email address is confirmed and your ${serverName} account is ready to use.`,
      action: { label: "Continue", href: next },
    };
  }
  if (result === "expired") {
    return {
      result,
      title: "This link has expired",
      body: "Confirmation links work for 24 hours. Sign in with your email and password and we will send you a new one.",
      action: { label: "Sign in", href: signInHref(next) },
    };
  }
  return {
    result,
    title: "This link did not work",
    body: "It may have been cut short when it was copied. Open it straight from the email, or sign in with your email and password and we will send you a new one.",
    action: { label: "Sign in", href: signInHref(next) },
  };
}
