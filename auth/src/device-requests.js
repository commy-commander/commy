import { createHmac } from "node:crypto";
import * as z from "zod";
import { createAuthEndpoint, createAuthMiddleware, getIP, sessionMiddleware } from "better-auth/api";

const MODEL = "deviceRequest";
const TTL_MS = 15 * 60 * 1000;

export function networkTag(secret, ip) {
  if (!ip) return "";
  return createHmac("sha256", secret).update(`network\n${ip}`).digest("base64url");
}

export function sameNetwork(storedTag, currentTag) {
  if (!storedTag || !currentTag) return null;
  return storedTag === currentTag;
}

export const commyDeviceRequests = ({ secret }) => ({
  id: "commy-device-requests",
  schema: {
    [MODEL]: {
      fields: {
        userCode: { type: "string", required: true, unique: true },
        network: { type: "string", required: false },
        createdAt: { type: "date", required: true },
        expiresAt: { type: "date", required: true },
      },
    },
  },
  hooks: {
    after: [
      {
        matcher: (ctx) => ctx.path === "/device/code",
        handler: createAuthMiddleware(async (ctx) => {
          const returned = ctx.context.returned;
          const userCode = returned && typeof returned === "object" ? returned.user_code : undefined;
          if (typeof userCode !== "string") return;
          const now = new Date();
          const adapter = ctx.context.adapter;
          await adapter.deleteMany({ model: MODEL, where: [{ field: "expiresAt", operator: "lt", value: now }] });
          const ip = ctx.headers ? getIP(ctx.headers, ctx.context.options) : null;
          await adapter.create({
            model: MODEL,
            data: {
              userCode,
              network: networkTag(secret, ip),
              createdAt: now,
              expiresAt: new Date(now.getTime() + TTL_MS),
            },
          });
        }),
      },
    ],
  },
  endpoints: {
    deviceRequestContext: createAuthEndpoint(
      "/commy/device-request",
      {
        method: "GET",
        use: [sessionMiddleware],
        query: z.object({ user_code: z.string() }),
      },
      async (ctx) => {
        const row = await ctx.context.adapter.findOne({
          model: MODEL,
          where: [{ field: "userCode", value: ctx.query.user_code.toUpperCase() }],
        });
        if (!row) return ctx.json({ found: false, sameNetwork: null, requestedAt: null });
        const ip = ctx.headers ? getIP(ctx.headers, ctx.context.options) : null;
        return ctx.json({
          found: true,
          sameNetwork: sameNetwork(row.network, networkTag(secret, ip)),
          requestedAt: row.createdAt,
        });
      },
    ),
  },
});
