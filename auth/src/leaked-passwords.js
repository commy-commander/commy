import { createHash } from "node:crypto";
import { tryGetCurrentAuthEndpointContext } from "@better-auth/core/context";
import { APIError } from "better-auth/api";

const RANGE_API = "https://api.pwnedpasswords.com/range/";
const LOOKUP_TIMEOUT_MS = 3000;
export const NEW_PASSWORD_PATHS = ["/sign-up/email", "/change-password", "/reset-password", "/commy/password/set"];
export const LEAKED_PASSWORD_MESSAGE =
  "This password has shown up in a known data breach, so others may try it. Choose a different password.";

export function passwordHashParts(password) {
  const hex = createHash("sha1").update(password, "utf8").digest("hex").toUpperCase();
  return { prefix: hex.slice(0, 5), suffix: hex.slice(5) };
}

export function rangeListsSuffix(body, suffix) {
  for (const line of String(body).split(/\r?\n/)) {
    const [entry, count] = line.trim().split(":");
    if (entry?.toUpperCase() === suffix && Number(count) > 0) return true;
  }
  return false;
}

export function pwnedPasswordsLookup({ fetch = globalThis.fetch, timeoutMs = LOOKUP_TIMEOUT_MS } = {}) {
  return async (password) => {
    const { prefix, suffix } = passwordHashParts(password);
    const response = await fetch(`${RANGE_API}${prefix}`, {
      headers: { "Add-Padding": "true", "User-Agent": "commy-auth" },
      signal: AbortSignal.timeout(timeoutMs),
    });
    if (!response.ok) throw new Error(`status ${response.status}`);
    return rangeListsSuffix(await response.text(), suffix);
  };
}

export async function isKnownLeak(isLeaked, password, log = console) {
  try {
    return await isLeaked(password);
  } catch (err) {
    log.warn(`[auth] leaked password check skipped: ${err?.message ?? err}`);
    return false;
  }
}

function choosesNewPassword() {
  return NEW_PASSWORD_PATHS.includes(tryGetCurrentAuthEndpointContext()?.path);
}

export const commyLeakedPasswords = ({ isLeaked, log = console }) => ({
  id: "commy-leaked-passwords",
  init(ctx) {
    const hash = ctx.password.hash;
    return {
      context: {
        password: {
          ...ctx.password,
          async hash(password) {
            if (choosesNewPassword() && (await isKnownLeak(isLeaked, password, log))) {
              throw new APIError("BAD_REQUEST", { code: "PASSWORD_COMPROMISED", message: LEAKED_PASSWORD_MESSAGE });
            }
            return hash(password);
          },
        },
      },
    };
  },
});
