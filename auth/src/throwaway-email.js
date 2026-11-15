import { readFileSync } from "node:fs";
import { APIError, createAuthMiddleware } from "better-auth/api";

const BUNDLED_LIST = new URL("../assets/throwaway-email-domains.txt", import.meta.url);
const MAGIC_LINK_PATH = "/sign-in/magic-link";
const CHANGE_EMAIL_PATH = "/change-email";

export function loadThrowawayDomains(path = BUNDLED_LIST) {
  const domains = new Set();
  for (const line of readFileSync(path, "utf8").split("\n")) {
    const domain = line.trim().toLowerCase();
    if (domain) domains.add(domain);
  }
  return domains;
}

export function emailDomain(email) {
  const text = String(email ?? "").trim().toLowerCase();
  const at = text.lastIndexOf("@");
  return at < 0 ? "" : text.slice(at + 1).replace(/\.+$/, "");
}

export function isThrowawayEmail(email, domains) {
  const labels = emailDomain(email).split(".");
  for (let i = 0; i < labels.length - 1; i++) {
    if (domains.has(labels.slice(i).join("."))) return true;
  }
  return false;
}

export function refuseThrowawayEmail(email, domains) {
  if (!domains || !isThrowawayEmail(email, domains)) return;
  throw new APIError("BAD_REQUEST", {
    code: "THROWAWAY_EMAIL",
    message: "That email address is from a throwaway email service. Use an address you plan to keep.",
  });
}

async function isNewAccount(ctx, email) {
  return !(await ctx.context.internalAdapter.findUserByEmail(String(email).toLowerCase()));
}

export const commyThrowawayEmails = ({ domains }) => ({
  id: "commy-throwaway-emails",
  hooks: {
    before: [
      {
        matcher: (ctx) => ctx.path === MAGIC_LINK_PATH,
        handler: createAuthMiddleware(async (ctx) => {
          const email = ctx.body?.email;
          if (email && isThrowawayEmail(email, domains) && (await isNewAccount(ctx, email))) refuseThrowawayEmail(email, domains);
        }),
      },
      {
        matcher: (ctx) => ctx.path === CHANGE_EMAIL_PATH,
        handler: createAuthMiddleware(async (ctx) => refuseThrowawayEmail(ctx.body?.newEmail, domains)),
      },
    ],
  },
});
