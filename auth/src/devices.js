import * as z from "zod";
import { APIError, createAuthEndpoint, sessionMiddleware } from "better-auth/api";
import { checkChallenge, linkMessage, makeChallenge, verifyNodeSignature } from "./proof.js";

export const MAX_DEVICES_PER_USER = 25;
const NODE_ID = /^[0-9a-f]{64}$/;
const nameOlderAppsSendAndWeIgnore = z.string().optional();

export function normalizeNodeId(value) {
  const id = String(value ?? "").trim().toLowerCase();
  return NODE_ID.test(id) ? id : null;
}

function requireNodeId(value) {
  const nodeId = normalizeNodeId(value);
  if (!nodeId) {
    throw new APIError("BAD_REQUEST", { message: "nodeId must be a 64 character hex iroh node id" });
  }
  return nodeId;
}

export const DEVICE_LABEL = "Commy desktop";

export function toPublicDevice(row) {
  return {
    id: row.id,
    nodeId: row.nodeId,
    name: DEVICE_LABEL,
    createdAt: row.createdAt,
    lastSeenAt: row.lastSeenAt,
  };
}

export async function dropDeviceNames(pool) {
  await pool.query('ALTER TABLE IF EXISTS "linkedDevice" DROP COLUMN IF EXISTS "name"');
}

const MODEL = "linkedDevice";

async function endDeviceSession(adapter, userId, sessionId, currentSessionId) {
  if (!sessionId || sessionId === currentSessionId) return false;
  await adapter.deleteMany({
    model: "session",
    where: [
      { field: "id", value: sessionId },
      { field: "userId", value: userId },
    ],
  });
  return true;
}

export const commyDevices = ({ secret, serverUrl }) => ({
  id: "commy-devices",
  schema: {
    [MODEL]: {
      fields: {
        nodeId: { type: "string", required: true, unique: true },
        userId: {
          type: "string",
          required: true,
          references: { model: "user", field: "id", onDelete: "cascade" },
          index: true,
        },
        createdAt: { type: "date", required: true },
        lastSeenAt: { type: "date", required: true },
        sessionId: { type: "string", required: false },
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
        const nodeId = requireNodeId(ctx.body.nodeId);
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
          name: nameOlderAppsSendAndWeIgnore,
          challenge: z.string(),
          signature: z.string(),
        }),
      },
      async (ctx) => {
        const userId = ctx.context.session.user.id;
        const nodeId = requireNodeId(ctx.body.nodeId);
        const { challenge, signature } = ctx.body;
        if (
          !checkChallenge(secret, userId, nodeId, challenge) ||
          !verifyNodeSignature(nodeId, linkMessage(serverUrl, nodeId, challenge), signature)
        ) {
          throw new APIError("FORBIDDEN", { message: "Could not verify that this app holds the node key" });
        }
        const now = new Date();
        const sessionId = ctx.context.session.session.id;
        const adapter = ctx.context.adapter;

        const existing = await adapter.findOne({ model: MODEL, where: [{ field: "nodeId", value: nodeId }] });
        if (!existing || existing.userId !== userId) {
          const count = await adapter.count({ model: MODEL, where: [{ field: "userId", value: userId }] });
          if (count >= MAX_DEVICES_PER_USER) {
            throw new APIError("BAD_REQUEST", { message: `An account can link at most ${MAX_DEVICES_PER_USER} devices` });
          }
        }
        if (existing) {
          const updated = await adapter.update({
            model: MODEL,
            where: [{ field: "id", value: existing.id }],
            update: { userId, lastSeenAt: now, sessionId },
          });
          return ctx.json({ device: toPublicDevice(updated ?? { ...existing, userId, lastSeenAt: now }) });
        }

        const created = await adapter.create({
          model: MODEL,
          data: { nodeId, userId, createdAt: now, lastSeenAt: now, sessionId },
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
        const adapter = ctx.context.adapter;
        const owned = [
          { field: "id", value: ctx.body.id },
          { field: "userId", value: userId },
        ];
        const device = await adapter.findOne({ model: MODEL, where: owned });
        if (!device) return ctx.json({ success: true, signedOut: false });
        await adapter.deleteMany({ model: MODEL, where: owned });
        const signedOut = await endDeviceSession(adapter, userId, device.sessionId, ctx.context.session.session.id);
        return ctx.json({ success: true, signedOut });
      },
    ),
  },
});
