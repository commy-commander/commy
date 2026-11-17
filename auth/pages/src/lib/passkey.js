import { browserSupportsWebAuthn, browserSupportsWebAuthnAutofill, startAuthentication, startRegistration } from "@simplewebauthn/browser";
import { authGet, authPost } from "./auth-client.js";

export const passkeysWork = () => typeof window !== "undefined" && browserSupportsWebAuthn();

export async function addPasskey(name) {
  const optionsJSON = await authGet(
    "/passkey/generate-register-options",
  );
  const response = await startRegistration({ optionsJSON });
  await authPost("/passkey/verify-registration", { response, ...(name ? { name } : {}) });
}

export async function signInWithPasskey() {
  const optionsJSON = await authGet(
    "/passkey/generate-authenticate-options",
  );
  const response = await startAuthentication({ optionsJSON });
  await authPost("/passkey/verify-authentication", { response });
}

export async function signInWithPasskeyAutofill() {
  if (!(await browserSupportsWebAuthnAutofill())) return false;
  const optionsJSON = await authGet(
    "/passkey/generate-authenticate-options",
  );
  const response = await startAuthentication({ optionsJSON, useBrowserAutofill: true });
  await authPost("/passkey/verify-authentication", { response });
  return true;
}

export function passkeyErrorMessage(err, fallback) {
  if (err instanceof Error && (err.name === "NotAllowedError" || err.name === "AbortError")) {
    return "The passkey prompt was closed before it finished.";
  }
  if (err instanceof Error && err.name === "InvalidStateError") {
    return "This device already has a passkey for your account.";
  }
  return err instanceof Error && err.message ? err.message : fallback;
}
