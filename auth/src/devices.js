// Better Auth plugin linking accounts to Commy desktop installs. A device is
// identified by its iroh node id: a public key, never a secret.
import * as z from "zod";
import { APIError, createAuthEndpoint, sessionMiddleware } from "better-auth/api";
import { checkChallenge, linkMessage, makeChallenge, verifyNodeSignature } from "./proof.js";

export const MAX_DEVICES_PER_USER = 25;
const NODE_ID = /^[0-9a-f]{64}$/;

export function normalizeNodeId(value) {
  const id = String(value ?? "").trim().toLowerCase();
  return NODE_ID.test(id) ? id : null;
}

export function normalizeDeviceName(value) {
  const name = String(value ?? "")
    .replace(/[\u0000-\u001f\u007f]/g, "")
    .trim()
    .slice(0, 64);
  return name || "Commy desktop";
}

export function toPublicDevice(row) {
  return {
    id: row.id,
    nodeId: row.nodeId,
    name: row.name,
    createdAt: row.createdAt,
    lastSeenAt: row.lastSeenAt,
  };
}

const MODEL = "linkedDevice";

export const commyDevices = ({ secret, serverUrl }) => ({
  id: "commy-devices",
  schema: {
    [MODEL]: {
      fields: {
        nodeId: { type: "string", required: true, unique: true },
        name: { type: "string", required: true },
        userId: {
          type: "string",
          required: true,
          references: { model: "user", field: "id", onDelete: "cascade" },
          index: true,
        },
        createdAt: { type: "date", required: true },
        lastSeenAt: { type: "date", required: true },
      },
    },
  },
  endpoints: {
    listCommyDevices: createAuthEndpoint(
      "/commy/devices",
      { method: "GET", use: [sessionMiddleware] },
      async (ctx) => {
        const rows = await ctx.context.adapter.findMany({
          model: MODEL,
          where: [{ field: "userId", value: ctx.context.session.user.id }],
          sortBy: { field: "lastSeenAt", direction: "desc" },
        });
        return ctx.json({ devices: rows.map(toPublicDevice) });
      },
    ),

    deviceLinkChallenge: createAuthEndpoint(
      "/commy/devices/challenge",
      {
        method: "POST",
        use: [sessionMiddleware],
        body: z.object({ nodeId: z.string() }),
      },
      async (ctx) => {
        const nodeId = normalizeNodeId(ctx.body.nodeId);
        if (!nodeId) {
          throw new APIError("BAD_REQUEST", { message: "nodeId must be a 64 character hex iroh node id" });
        }
        const challenge = makeChallenge(secret, ctx.context.session.user.id, nodeId);
        return ctx.json({ challenge, message: linkMessage(serverUrl, nodeId, challenge) });
      },
    ),

    registerCommyDevice: createAuthEndpoint(
      "/commy/devices/register",
      {
        method: "POST",
        use: [sessionMiddleware],
        body: z.object({
          nodeId: z.string(),
          name: z.string().optional(),
          challenge: z.string(),
          signature: z.string(),
        }),
      },
      async (ctx) => {
        const userId = ctx.context.session.user.id;
        const nodeId = normalizeNodeId(ctx.body.nodeId);
        if (!nodeId) {
          throw new APIError("BAD_REQUEST", { message: "nodeId must be a 64 character hex iroh node id" });
        }
        const { challenge, signature } = ctx.body;
        if (
          !checkChallenge(secret, userId, nodeId, challenge) ||
          !verifyNodeSignature(nodeId, linkMessage(serverUrl, nodeId, challenge), signature)
        ) {
          throw new APIError("FORBIDDEN", { message: "Could not verify that this app holds the node key" });
        }
        const name = normalizeDeviceName(ctx.body.name);
        const now = new Date();
        const adapter = ctx.context.adapter;

        const existing = await adapter.findOne({ model: MODEL, where: [{ field: "nodeId", value: nodeId }] });
        if (!existing || existing.userId !== userId) {
          const count = await adapter.count({ model: MODEL, where: [{ field: "userId", value: userId }] });
          if (count >= MAX_DEVICES_PER_USER) {
            throw new APIError("BAD_REQUEST", { message: `An account can link at most ${MAX_DEVICES_PER_USER} devices` });
          }
        }
        if (existing) {
          // The signature proves key possession, so a device signing in to a new
          // account moves over rather than staying with the old one
          const updated = await adapter.update({
            model: MODEL,
            where: [{ field: "id", value: existing.id }],
            update: { name, userId, lastSeenAt: now },
          });
          return ctx.json({ device: toPublicDevice(updated ?? { ...existing, name, userId, lastSeenAt: now }) });
        }

        const created = await adapter.create({
          model: MODEL,
          data: { nodeId, name, userId, createdAt: now, lastSeenAt: now },
        });
        return ctx.json({ device: toPublicDevice(created) });
      },
    ),

    removeCommyDevice: createAuthEndpoint(
      "/commy/devices/remove",
      {
        method: "POST",
        use: [sessionMiddleware],
        body: z.object({ id: z.string() }),
      },
      async (ctx) => {
        const userId = ctx.context.session.user.id;
        await ctx.context.adapter.deleteMany({
          model: MODEL,
          where: [
            { field: "id", value: ctx.body.id },
            { field: "userId", value: userId },
          ],
        });
        return ctx.json({ success: true });
      },
    ),
  },
});
