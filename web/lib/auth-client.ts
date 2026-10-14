// Browser side calls to the auth service, mounted at /api/auth on this origin.

export class AuthError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly code: string | undefined,
  ) {
    super(message);
  }
}

async function parse<T>(res: Response): Promise<T> {
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    throw new AuthError(
      data.message || data.error_description || "Something went wrong, please try again.",
      res.status,
      data.code,
    );
  }
  return data as T;
}

export async function authPost<T = unknown>(path: string, body: object = {}, headers: Record<string, string> = {}): Promise<T> {
  const res = await fetch(`/api/auth${path}`, {
    method: "POST",
    credentials: "same-origin",
    headers: { "content-type": "application/json", ...headers },
    body: JSON.stringify(body),
  });
  return parse<T>(res);
}

export async function authGet<T = unknown>(path: string): Promise<T> {
  const res = await fetch(`/api/auth${path}`, { credentials: "same-origin" });
  return parse<T>(res);
}
