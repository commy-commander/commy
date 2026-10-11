// Browser side calls to the auth service, mounted at /api/auth on this origin.

export class AuthError extends Error {}

export async function authPost<T = unknown>(path: string, body: object = {}, headers: Record<string, string> = {}): Promise<T> {
  const res = await fetch(`/api/auth${path}`, {
    method: "POST",
    credentials: "same-origin",
    headers: { "content-type": "application/json", ...headers },
    body: JSON.stringify(body),
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    throw new AuthError(data.message || data.error_description || "Something went wrong, please try again.");
  }
  return data as T;
}

export async function authGet<T = unknown>(path: string): Promise<T> {
  const res = await fetch(`/api/auth${path}`, { credentials: "same-origin" });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    throw new AuthError(data.message || data.error_description || "Something went wrong, please try again.");
  }
  return data as T;
}
