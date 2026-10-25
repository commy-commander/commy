import * as z from "zod";
import { APIError, createAuthEndpoint, getSessionFromCtx } from "better-auth/api";
import { findInvite, isPermanent, linkCodeSpellings } from "./invites.js";

const MODEL = "directoryListing";
const INVITE_MODEL = "invite";
export const CATEGORIES = ["gaming", "music", "art", "tech", "science", "education", "friends", "languages", "sports", "other"];
export const MEMBER_BUCKETS = [10, 25, 50, 100, 250, 500, 1000];
export const NAME_MAX = 48;
export const DESCRIPTION_MAX = 300;
export const MAX_PER_ACCOUNT = 5;
export const PAGE_SIZE = 24;
export const QUERY_MAX = 80;
export const MAX_WORDS = 6;
const MAX_SCAN = 5000;
const KEY = /^[A-Za-z0-9_-]{22,43}$/;
const CONTROL = /[\u0000-\u001f\u007f]/g;

function tidy(value) {
  return String(value ?? "").replace(CONTROL, " ").replace(/\s+/g, " ").trim();
}

export function cleanName(value) {
  const name = tidy(value);
  return name.length >= 1 && name.length <= NAME_MAX ? name : null;
}

export function cleanDescription(value) {
  const description = String(value ?? "").replace(/[^\S\n]+/g, " ").replace(/\n{3,}/g, "\n\n").trim();
  return description.length <= DESCRIPTION_MAX && !/[\u0000-\u0008\u000b-\u001f\u007f]/.test(description) ? description : null;
}

export function validCategory(value) {
  const category = String(value ?? "").trim().toLowerCase();
  return CATEGORIES.includes(category) ? category : null;
}

export function validBucket(value) {
  return MEMBER_BUCKETS.includes(value) ? value : null;
}

export function validKey(value) {
  const key = String(value ?? "").trim();
  return KEY.test(key) ? key : null;
}

export function searchWords(value) {
  return tidy(value).toLowerCase().slice(0, QUERY_MAX).split(" ").filter(Boolean).slice(0, MAX_WORDS);
}

export function matchesWords(row, words) {
  const haystack = `${row.name} ${row.description}`.toLowerCase();
  return words.every((word) => haystack.includes(word));
}

export function parseCursor(value) {
  if (value === undefined || value === "") return 0;
  return /^\d{1,7}$/.test(String(value)) ? Number(value) : null;
}

export function sortListings(rows) {
  return [...rows].sort(
    (a, b) =>
      b.members - a.members ||
      new Date(b.updatedAt).getTime() - new Date(a.updatedAt).getTime() ||
      (a.code < b.code ? -1 : a.code > b.code ? 1 : 0),
  );
}

export function publicListing(row) {
  return {
    code: row.code,
    key: row.key,
    name: row.name,
    description: row.description,
    category: row.category,
    members: row.members,
  };
}

async function liveInviteCodes(adapter, codes) {
  if (!codes.length) return new Set();
  const rows = await adapter.findMany({
    model: INVITE_MODEL,
    where: [{ field: "code", operator: "in", value: codes }],
    limit: codes.length,
  });
  const now = new Date();
  return new Set(rows.filter((r) => new Date(r.expiresAt) > now).map((r) => r.code));
}

export const commyDirectory = () => ({
  id: "commy-directory",
  schema: {
    [MODEL]: {
      fields: {
        code: { type: "string", required: true, unique: true },
        key: { type: "string", required: true },
        name: { type: "string", required: true },
        description: { type: "string", required: true },
        category: { type: "string", required: true, index: true },
        members: { type: "number", required: true, index: true },
        createdBy: { type: "string", required: true, index: true },
        createdAt: { type: "date", required: true },
        updatedAt: { type: "date", required: true, index: true },
      },
    },
  },
  endpoints: {
    publishCommyListing: createAuthEndpoint(
      "/commy/directory/publish",
      {
        method: "POST",
        body: z.object({
          code: z.string(),
          key: z.string(),
          name: z.string(),
          description: z.string().optional(),
          category: z.string(),
          members: z.number(),
        }),
      },
      async (ctx) => {
        const session = await getSessionFromCtx(ctx);
        if (!session) throw new APIError("UNAUTHORIZED", { message: "Sign in to list a server in Discover" });
        if (!linkCodeSpellings(ctx.body.code).length) throw new APIError("BAD_REQUEST", { message: "That is not an invite code" });
        const key = validKey(ctx.body.key);
        if (!key) throw new APIError("BAD_REQUEST", { message: "That invite link is missing its key" });
        const name = cleanName(ctx.body.name);
        if (!name) throw new APIError("BAD_REQUEST", { message: `The server name must be 1 to ${NAME_MAX} characters` });
        const description = cleanDescription(ctx.body.description ?? "");
        if (description === null) {
          throw new APIError("BAD_REQUEST", { message: `The description can hold up to ${DESCRIPTION_MAX} characters` });
        }
        const category = validCategory(ctx.body.category);
        if (!category) throw new APIError("BAD_REQUEST", { message: "Pick a category from the list" });
        const members = validBucket(ctx.body.members);
        if (members === null) throw new APIError("BAD_REQUEST", { message: "That member count is not a known size" });

        const adapter = ctx.context.adapter;
        const invite = await findInvite(adapter, ctx.body.code);
        if (!invite || new Date(invite.expiresAt) < new Date() || invite.createdBy !== session.user.id) {
          throw new APIError("NOT_FOUND", { message: "Only your own invite links can be listed" });
        }
        if (!isPermanent(invite.expiresAt)) {
          throw new APIError("BAD_REQUEST", { message: "A listing needs an invite link that never expires" });
        }

        const code = invite.code;
        const now = new Date();
        const fields = { key, name, description, category, members, updatedAt: now };
        const existing = await adapter.findOne({ model: MODEL, where: [{ field: "code", value: code }] });
        if (existing && existing.createdBy !== session.user.id) {
          throw new APIError("FORBIDDEN", { message: "Someone else listed this invite" });
        }
        if (existing) {
          const updated = await adapter.update({ model: MODEL, where: [{ field: "id", value: existing.id }], update: fields });
          return ctx.json(publicListing(updated ?? { ...existing, ...fields }));
        }
        const mine = await adapter.count({ model: MODEL, where: [{ field: "createdBy", value: session.user.id }] });
        if (mine >= MAX_PER_ACCOUNT) {
          throw new APIError("FORBIDDEN", { message: `You can list up to ${MAX_PER_ACCOUNT} servers. Unlist one first.` });
        }
        const created = await adapter.create({
          model: MODEL,
          data: { code, createdBy: session.user.id, createdAt: now, ...fields },
        });
        return ctx.json(publicListing(created));
      },
    ),

    unlistCommyListing: createAuthEndpoint(
      "/commy/directory/unlist",
      { method: "POST", body: z.object({ code: z.string() }) },
      async (ctx) => {
        const session = await getSessionFromCtx(ctx);
        if (!session) throw new APIError("UNAUTHORIZED", { message: "Sign in to change Discover listings" });
        const adapter = ctx.context.adapter;
        for (const code of linkCodeSpellings(ctx.body.code)) {
          const row = await adapter.findOne({ model: MODEL, where: [{ field: "code", value: code }] });
          if (row && row.createdBy === session.user.id) {
            await adapter.deleteMany({ model: MODEL, where: [{ field: "code", value: row.code }] });
          }
        }
        return ctx.json({ success: true });
      },
    ),

    listCommyDirectory: createAuthEndpoint(
      "/commy/directory/list",
      {
        method: "GET",
        query: z.object({ q: z.string().optional(), category: z.string().optional(), cursor: z.string().optional() }),
      },
      async (ctx) => {
        const category = ctx.query.category === undefined || ctx.query.category === "" ? null : validCategory(ctx.query.category);
        if (ctx.query.category && !category) throw new APIError("BAD_REQUEST", { message: "Unknown category" });
        const offset = parseCursor(ctx.query.cursor);
        if (offset === null) throw new APIError("BAD_REQUEST", { message: "That page is not valid" });
        const words = searchWords(ctx.query.q);

        const adapter = ctx.context.adapter;
        const rows = await adapter.findMany({
          model: MODEL,
          where: category ? [{ field: "category", value: category }] : [],
          limit: MAX_SCAN,
        });
        const matching = sortListings(rows.filter((row) => matchesWords(row, words)));
        const page = matching.slice(offset, offset + PAGE_SIZE);
        const live = await liveInviteCodes(adapter, page.map((row) => row.code));
        const gone = page.filter((row) => !live.has(row.code));
        for (const row of gone) await adapter.deleteMany({ model: MODEL, where: [{ field: "code", value: row.code }] });
        const listings = page.filter((row) => live.has(row.code)).map(publicListing);
        const next = offset + PAGE_SIZE;
        return ctx.json({ listings, nextCursor: next < matching.length ? String(next) : null });
      },
    ),
  },
});
