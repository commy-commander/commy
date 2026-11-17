import * as z from "zod";
import { APIError, createAuthEndpoint } from "better-auth/api";

export const SERVER_PROOF_PATH = "/commy/server-proof";
export const PROOF_PURPOSE = "server-proof";
const NONCE_HEX = /^[0-9a-f]{64}$/;

export function proveServer(serverKey, origin, nonce) {
  const challenge = String(nonce ?? "").toLowerCase();
  if (!NONCE_HEX.test(challenge)) return null;
  return { origin, nonce: challenge, key: serverKey.published, sig: serverKey.sign(PROOF_PURPOSE, origin, challenge) };
}

export const commyServerProof = ({ serverKey, origin }) => ({
  id: "commy-server-proof",
  endpoints: {
    serverProof: createAuthEndpoint(SERVER_PROOF_PATH, { method: "GET", query: z.object({ nonce: z.string() }) }, async (ctx) => {
      const proof = proveServer(serverKey, origin, ctx.query.nonce);
      if (!proof) throw new APIError("BAD_REQUEST", { message: "nonce must be 64 hex characters" });
      ctx.setHeader("Cache-Control", "no-store");
      return ctx.json(proof);
    }),
  },
});
