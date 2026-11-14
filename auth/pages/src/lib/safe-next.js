export function safeNext(value, fallback = "/account") {
  if (typeof value === "string" && value.length <= 200 && /^\/(?!\/)[^\\\s]*$/.test(value)) return value;
  return fallback;
}

export function signUpHref(next) {
  return next === "/account" ? "/signup" : `/signup?next=${encodeURIComponent(next)}`;
}

export function twoFactorHref(next) {
  return `/account/two-factor?next=${encodeURIComponent(safeNext(next))}`;
}
