import * as z from "zod";
import { createHmac, hkdfSync } from "node:crypto";
import { APIError, createAuthEndpoint, sessionMiddleware } from "better-auth/api";

const LOCK_ID = /^[0-9a-f]{32}$/;

export function normalizeLockId(value) {
  const id = String(value ?? "").trim().toLowerCase();
  return LOCK_ID.test(id) ? id : null;
}

export function folderKeyDeriver(secret) {
  const root = Buffer.from(hkdfSync("sha256", secret, "commy-folder-lock-v1", "folder keys", 32));
  return (userId, lockId) => createHmac("sha256", root).update(userId).update("\0").update(lockId).digest("hex");
}

export const commyFolderLock = ({ secret }) => {
  const folderKey = folderKeyDeriver(secret);
  return {
    id: "commy-folder-lock",
    endpoints: {
      commyFolderKey: createAuthEndpoint(
        "/commy/folder-key",
        { method: "POST", use: [sessionMiddleware], body: z.object({ lockId: z.string() }) },
        async (ctx) => {
          const lockId = normalizeLockId(ctx.body.lockId);
          if (!lockId) throw new APIError("BAD_REQUEST", { message: "lockId must be 32 hex characters" });
          return ctx.json({ key: folderKey(ctx.context.session.user.id, lockId) });
        },
      ),
    },
  };
};
