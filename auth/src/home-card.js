import * as z from "zod";
import { APIError, createAuthEndpoint, sessionMiddleware } from "better-auth/api";
import { movedAway } from "./moved.js";
import { verifyNodeSignature } from "./proof.js";

export const HOME_CARD_PATH = "/commy/home-card";
export const CARD_PURPOSE = "home-card";
export const CARD_DAYS = 30;
export const REQUEST_WINDOW_MS = 10 * 60 * 1000;
const DAY_MS = 24 * 60 * 60 * 1000;
const USER_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const KEY_HEX = /^[0-9a-f]{64}$/;

export function requestMessage(home, user, identity, at) {
  return `commy-home-request/v1\n${home}\n${user}\n${identity}\n${at}`;
}

export function cardPayload(user, identity, issued, expires) {
  return `${user}\n${identity}\n${issued}\n${expires}`;
}

export function issueCard(serverKey, home, user, identity, now = Date.now()) {
  const issued = now;
  const expires = now + CARD_DAYS * DAY_MS;
  const sig = serverKey.sign(CARD_PURPOSE, home, cardPayload(user, identity, issued, expires));
  return { home, user, identity, key: serverKey.published.public, issued, expires, sig };
}

export function checkCardRequest(home, { user, identity, at, proof }, now = Date.now()) {
  if (!USER_ID.test(user) || !KEY_HEX.test(identity)) return "user and identity must be a Commy user id and key";
  if (!Number.isSafeInteger(at) || Math.abs(now - at) > REQUEST_WINDOW_MS) return "the request is too old; check the device clock";
  if (!verifyNodeSignature(identity, requestMessage(home, user, identity, at), proof)) return "the identity key did not sign this request";
  return null;
}

export const commyHomeCard = ({ serverKey, home }) => ({
  id: "commy-home-card",
  endpoints: {
    issueHomeCard: createAuthEndpoint(
      HOME_CARD_PATH,
      {
        method: "POST",
        use: [sessionMiddleware],
        body: z.object({ user: z.string(), identity: z.string(), at: z.number(), proof: z.string() }),
      },
      async (ctx) => {
        const request = { ...ctx.body, user: ctx.body.user.toLowerCase(), identity: ctx.body.identity.toLowerCase() };
        const problem = checkCardRequest(home, request);
        if (problem) throw new APIError("BAD_REQUEST", { message: problem });
        const moved = await movedAway(ctx.context.adapter, home, request.identity);
        if (moved) throw new APIError("CONFLICT", { code: "ACCOUNT_MOVED", message: `This account moved to ${moved.to}.` });
        return ctx.json({ card: issueCard(serverKey, home, request.user, request.identity) });
      },
    ),
  },
});
