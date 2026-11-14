import { authPost } from "./auth-client.js";

export const ACCESS_COOKIE = "commy_access";
export const ACCESS_MAX_AGE = 60 * 60 * 24 * 90;

export function accessCookie(token, secure) {
  return `${ACCESS_COOKIE}=${token}; Max-Age=${ACCESS_MAX_AGE}; Path=/; SameSite=Lax${secure ? "; Secure" : ""}`;
}

export async function unlock(code) {
  const { token } = await authPost("/commy/access", { code });
  if (token) document.cookie = accessCookie(token, window.location.protocol === "https:");
}
