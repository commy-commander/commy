// Cookie consent, browser side. Nothing optional may load, and nothing is
// written to the device, until the visitor makes a choice in the banner.
// The choice itself is the only thing stored, and only after the click.

export type Consent = "accepted" | "rejected";

export const CONSENT_COOKIE = "commy_consent";
export const CONSENT_MAX_AGE = 60 * 60 * 24 * 180;
const CHANGED = "commy-consent-changed";
const REOPEN = "commy-consent-open";

export function parseConsent(cookieHeader: string | null | undefined): Consent | null {
  for (const part of (cookieHeader ?? "").split(";")) {
    const [name, ...rest] = part.trim().split("=");
    if (name !== CONSENT_COOKIE) continue;
    const value = rest.join("=");
    return value === "accepted" || value === "rejected" ? value : null;
  }
  return null;
}

export function readConsent(): Consent | null {
  if (typeof document === "undefined") return null;
  return parseConsent(document.cookie);
}

export function saveConsent(value: Consent) {
  const secure = window.location.protocol === "https:" ? "; Secure" : "";
  document.cookie = `${CONSENT_COOKIE}=${value}; Max-Age=${CONSENT_MAX_AGE}; Path=/; SameSite=Lax${secure}`;
  window.dispatchEvent(new CustomEvent(CHANGED, { detail: value }));
}

export function onConsentChange(fn: (value: Consent) => void): () => void {
  const handler = (e: Event) => fn((e as CustomEvent<Consent>).detail);
  window.addEventListener(CHANGED, handler);
  return () => window.removeEventListener(CHANGED, handler);
}

export function openConsentSettings() {
  window.dispatchEvent(new Event(REOPEN));
}

export function onConsentSettingsOpen(fn: () => void): () => void {
  window.addEventListener(REOPEN, fn);
  return () => window.removeEventListener(REOPEN, fn);
}
