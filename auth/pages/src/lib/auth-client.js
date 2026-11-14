export class AuthError extends Error {
  constructor(message, status, code) {
    super(message);
    this.status = status;
    this.code = code;
  }
}

async function parse(res) {
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    throw new AuthError(
      data.message || data.error_description || "Something went wrong, please try again.",
      res.status,
      data.code,
    );
  }
  return data;
}

export async function authPost(path, body = {}, headers = {}) {
  const res = await fetch(`/api/auth${path}`, {
    method: "POST",
    credentials: "same-origin",
    headers: { "content-type": "application/json", ...headers },
    body: JSON.stringify(body),
  });
  return parse(res);
}

export async function authGet(path) {
  const res = await fetch(`/api/auth${path}`, { credentials: "same-origin" });
  return parse(res);
}

export const SIGN_IN_AGAIN = "For your safety, sign in again before making this change.";

export function isStaleSession(err) {
  return err instanceof AuthError && (err.code === "SESSION_NOT_FRESH" || err.code === "SESSION_EXPIRED");
}

export async function signOutAndReturn(path) {
  await authPost("/sign-out").catch(() => undefined);
  window.location.href = path;
}
