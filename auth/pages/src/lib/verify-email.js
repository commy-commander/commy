const HOME = "/";
const ACCOUNT = "/account";

function readResult(value) {
  return value === "confirmed" || value === "expired" ? value : "invalid";
}

function signInHref(next) {
  return next === ACCOUNT ? ACCOUNT : `${ACCOUNT}?next=${encodeURIComponent(next)}`;
}

export function verifyEmailOutcome(rawResult, safeNext, serverName = "Commy") {
  const result = readResult(rawResult);
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
