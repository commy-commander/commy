import { APIError, createAuthMiddleware, getOAuthState } from "better-auth/api";

const MAGIC_LINK_PATH = "/sign-in/magic-link";
const MAGIC_LINK_CONFIRMATION_TTL_MS = 60 * 60 * 1000;

const said = (data) => data?.adultConfirmed === true;

function magicLinkIdentifier(email) {
  return `commy-adult:${String(email ?? "").trim().toLowerCase()}`;
}

async function confirmedThroughOAuth() {
  try {
    return said(await getOAuthState());
  } catch {
    return false;
  }
}

async function confirmedThroughMagicLink(email, ctx) {
  const adapter = ctx?.context?.internalAdapter;
  if (!adapter || !email) return false;
  return Boolean(await adapter.consumeVerificationValue(magicLinkIdentifier(email)));
}

export async function confirmedAdult(user, ctx) {
  const body = ctx?.body;
  if (said(body) || said(body?.additionalData)) return true;
  if (await confirmedThroughOAuth()) return true;
  return confirmedThroughMagicLink(user?.email, ctx);
}

export async function recordAdultConfirmation(user, ctx, { required, termsVersion }) {
  if (await confirmedAdult(user, ctx)) {
    const now = new Date();
    return { ...user, adultConfirmedAt: now, termsAcceptedAt: now, termsVersion: termsVersion ?? null };
  }
  if (required) {
    throw new APIError("FORBIDDEN", {
      code: "ADULT_CONFIRMATION_REQUIRED",
      message: "Confirm that you are 18 or older to create an account.",
    });
  }
  return user;
}

export const adultConfirmationField = {
  adultConfirmedAt: { type: "date", required: false, input: false },
  termsAcceptedAt: { type: "date", required: false, input: false },
  termsVersion: { type: "string", required: false, input: false },
};

export const commyAdultConfirmation = () => ({
  id: "commy-adult-confirmation",
  hooks: {
    before: [
      {
        matcher: (ctx) => ctx.path === MAGIC_LINK_PATH,
        handler: createAuthMiddleware(async (ctx) => {
          if (!said(ctx.body?.metadata) || !ctx.body?.email) return;
          await ctx.context.internalAdapter.createVerificationValue({
            identifier: magicLinkIdentifier(ctx.body.email),
            value: "1",
            expiresAt: new Date(Date.now() + MAGIC_LINK_CONFIRMATION_TTL_MS),
          });
        }),
      },
    ],
  },
});
