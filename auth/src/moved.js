import * as z from "zod";
import { APIError, createAuthEndpoint, sessionMiddleware } from "better-auth/api";
import { isHomeOrigin } from "./home-keys.js";
import { verifyNodeSignature } from "./proof.js";

export const MOVED_PATH = "/commy/moved";
export const MOVED_SET_PATH = "/commy/moved/set";
export const REQUEST_WINDOW_MS = 10 * 60 * 1000;
export const KEPT_FOR_MS = 365 * 24 * 60 * 60 * 1000;
const MODEL = "commyMove";
const USER_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const KEY_HEX = /^[0-9a-f]{64}$/;

export function moveMessage(from, to, user, identity, at) {
  return `commy-moved/v1\n${from}\n${to}\n${user}\n${identity}\n${at}`;
}

export function checkMoveRequest(home, { to, user, identity, at, proof }, now = Date.now()) {
  if (!USER_ID.test(user) || !KEY_HEX.test(identity)) return "user and identity must be a Commy user id and key";
  if (to !== home && !isHomeOrigin(to)) return "the new home must be an https address on a public domain";
  if (!Number.isSafeInteger(at) || Math.abs(now - at) > REQUEST_WINDOW_MS) return "the request is too old; check the device clock";
  if (!verifyNodeSignature(identity, moveMessage(home, to, user, identity, at), proof)) return "the identity key did not sign this move";
  return null;
}

export function pointerOf(home, row, now = Date.now()) {
  if (!row || now - Number(row.at) > KEPT_FOR_MS) return null;
  return { from: home, to: row.toHome, user: row.commyUser, identity: row.identity, at: Number(row.at), proof: row.proof };
}

export async function widenMoveTimes(pool) {
  const { rows } = await pool.query(
    "SELECT data_type FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = 'commyMove' AND column_name = 'at'",
  );
  if (rows[0]?.data_type !== "integer") return false;
  await pool.query('ALTER TABLE "commyMove" ALTER COLUMN "at" TYPE bigint');
  return true;
}

async function findMove(adapter, identity) {
  return adapter.findOne({ model: MODEL, where: [{ field: "identity", value: identity }] });
}

export const commyMoved = ({ home, now = () => Date.now() }) => ({
  id: "commy-moved",
  schema: {
    [MODEL]: {
      fields: {
        identity: { type: "string", required: true, unique: true },
        userId: { type: "string", required: true, references: { model: "user", field: "id", onDelete: "cascade" }, index: true },
        commyUser: { type: "string", required: true },
        toHome: { type: "string", required: true },
        at: { type: "number", bigint: true, required: true },
        proof: { type: "string", required: true },
      },
    },
  },
  endpoints: {
    movedTo: createAuthEndpoint(MOVED_PATH, { method: "GET", query: z.object({ identity: z.string() }) }, async (ctx) => {
      const identity = ctx.query.identity.toLowerCase();
      if (!KEY_HEX.test(identity)) throw new APIError("BAD_REQUEST", { message: "identity must be a Commy identity key" });
      const pointer = pointerOf(home, await findMove(ctx.context.adapter, identity), now());
      if (!pointer) throw new APIError("NOT_FOUND", { message: "this identity has not moved from here" });
      return ctx.json({ moved: pointer });
    }),
    setMovedTo: createAuthEndpoint(
      MOVED_SET_PATH,
      {
        method: "POST",
        use: [sessionMiddleware],
        body: z.object({ to: z.string(), user: z.string(), identity: z.string(), at: z.number(), proof: z.string() }),
      },
      async (ctx) => {
        const request = { ...ctx.body, user: ctx.body.user.toLowerCase(), identity: ctx.body.identity.toLowerCase(), to: ctx.body.to.trim().toLowerCase() };
        const problem = checkMoveRequest(home, request, now());
        if (problem) throw new APIError("BAD_REQUEST", { message: problem });
        const adapter = ctx.context.adapter;
        const existing = await findMove(adapter, request.identity);
        if (request.to === home) {
          if (existing) await adapter.deleteMany({ model: MODEL, where: [{ field: "identity", value: request.identity }] });
          return ctx.json({ moved: null });
        }
        const fields = { userId: ctx.context.session.user.id, commyUser: request.user, toHome: request.to, at: request.at, proof: request.proof };
        if (existing) {
          if (Number(existing.at) >= request.at) throw new APIError("CONFLICT", { message: "a newer move is already recorded" });
          await adapter.update({ model: MODEL, where: [{ field: "identity", value: request.identity }], update: fields });
        } else {
          await adapter.create({ model: MODEL, data: { identity: request.identity, ...fields } });
        }
        return ctx.json({ moved: pointerOf(home, { identity: request.identity, ...fields }, now()) });
      },
    ),
  },
});
