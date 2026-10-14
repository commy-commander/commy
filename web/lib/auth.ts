import { headers } from "next/headers";

// The auth service as seen from inside the compose network. The browser reaches
// the same service at /api/auth on this site through the reverse proxy.
const AUTH_INTERNAL_URL = (process.env.AUTH_INTERNAL_URL ?? "http://auth:3001").replace(/\/+$/, "");

export type SessionUser = {
  id: string;
  name: string;
  email: string;
  image?: string | null;
};

export type LinkedDevice = {
  id: string;
  nodeId: string;
  name: string;
  createdAt: string;
  lastSeenAt: string;
};

export type Providers = {
  google: boolean;
  twitch: boolean;
  magicLink: boolean;
  passwordReset: boolean;
  emailPassword: boolean;
  turnstileSiteKey: string | null;
};

export type DeviceRequestContext = {
  found: boolean;
  sameNetwork: boolean | null;
  requestedAt: string | null;
};

async function authFetch<T>(path: string): Promise<T | null> {
  const incoming = headers();
  const forwarded: Record<string, string> = { cookie: incoming.get("cookie") ?? "" };
  // Pass the visitor's address along so per-IP rate limits apply to them, not to this site
  const forwardedFor = incoming.get("x-forwarded-for");
  if (forwardedFor) forwarded["x-forwarded-for"] = forwardedFor;
  try {
    const res = await fetch(`${AUTH_INTERNAL_URL}${path}`, {
      headers: forwarded,
      cache: "no-store",
    });
    if (!res.ok) return null;
    return (await res.json()) as T;
  } catch {
    return null;
  }
}

export async function getSessionUser(): Promise<SessionUser | null> {
  const data = await authFetch<{ user: SessionUser } | null>("/api/auth/get-session");
  return data?.user ?? null;
}

export async function listDevices(): Promise<LinkedDevice[]> {
  const data = await authFetch<{ devices: LinkedDevice[] }>("/api/auth/commy/devices");
  return data?.devices ?? [];
}

export async function getProviders(): Promise<Providers> {
  const data = await authFetch<{ providers: Omit<Providers, "turnstileSiteKey">; turnstileSiteKey: string | null }>(
    "/.well-known/commy-auth.json",
  );
  return {
    google: data?.providers.google ?? false,
    twitch: data?.providers.twitch ?? false,
    magicLink: data?.providers.magicLink ?? false,
    passwordReset: data?.providers.passwordReset ?? false,
    emailPassword: data?.providers.emailPassword ?? true,
    turnstileSiteKey: data?.turnstileSiteKey ?? null,
  };
}

export type ResolvedInvite = {
  code: string;
  url: string;
  serverName: string;
  expiresAt: string;
};

export async function resolveInvite(code: string): Promise<ResolvedInvite | null> {
  const data = await authFetch<ResolvedInvite & { ticket?: string }>(
    `/api/auth/commy/invites/resolve?code=${encodeURIComponent(code)}`,
  );
  if (!data) return null;
  // The page never needs the raw ticket; the app fetches it itself
  return { code: data.code, url: data.url, serverName: data.serverName, expiresAt: data.expiresAt };
}
