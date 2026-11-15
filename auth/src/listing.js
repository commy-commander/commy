import { createHash, createHmac, randomBytes, timingSafeEqual } from "node:crypto";
import * as z from "zod";
import { APIError, createAuthEndpoint } from "better-auth/api";
import { contactKind } from "./listing-checks.js";

export const MODEL = "listedServer";
export const APPLY_PATH = "/commy/listing/apply";
export const CONFIRM_PATH = "/commy/listing/confirm";
export const REVIEW_PATH = "/commy/listing/review";
export const DECIDE_PATH = "/commy/listing/decide";
export const APPLY_PAGE = "/servers/apply";
export const REVIEW_PAGE = "/servers/review";
export const STATUS = { confirming: "confirming", review: "review", listed: "listed", failing: "failing", delisted: "delisted", rejected: "rejected" };
export const SHOWN_STATUSES = [STATUS.listed, STATUS.failing];
const BUSY_STATUSES = [STATUS.review, STATUS.listed, STATUS.failing];
const DAY_MS = 24 * 60 * 60 * 1000;
const CONFIRM_DAYS = 7;
const REVIEW_DAYS = 30;
const MAX_NAME = 60;
const MAX_ABOUT = 140;
const TOKEN = /^[A-Za-z0-9_-]{20,200}$/;

const hash = (token) => createHash("sha256").update(token).digest("base64url");

export function cleanText(value, max) {
  const text = String(value ?? "").replace(/[\u0000-\u001f\u007f<>]/g, "").replace(/\s+/g, " ").trim();
  return text ? text.slice(0, max) : null;
}

export function reviewToken(secret, row, now = Date.now()) {
  const payload = Buffer.from(JSON.stringify({ o: row.origin, a: new Date(row.appliedAt).getTime(), e: now + REVIEW_DAYS * DAY_MS })).toString("base64url");
  const mac = createHmac("sha256", secret).update(`listing-review\n${payload}`).digest("base64url");
  return `${payload}.${mac}`;
}

export function openReviewToken(secret, token, now = Date.now()) {
  const [payload, mac] = String(token ?? "").split(".");
  if (!payload || !mac) return null;
  const expected = createHmac("sha256", secret).update(`listing-review\n${payload}`).digest();
  const given = Buffer.from(mac, "base64url");
  if (given.length !== expected.length || !timingSafeEqual(given, expected)) return null;
  try {
    const { o, a, e } = JSON.parse(Buffer.from(payload, "base64url").toString("utf8"));
    return typeof o === "string" && Number.isSafeInteger(a) && Number.isSafeInteger(e) && e > now ? { origin: o, appliedAt: a } : null;
  } catch {
    return null;
  }
}

export function rowFromCheck(result, form, now) {
  return {
    origin: result.origin,
    name: cleanText(form.name, MAX_NAME) ?? cleanText(result.name, MAX_NAME) ?? new URL(result.origin).hostname,
    about: cleanText(form.about, MAX_ABOUT),
    kinds: result.kinds.join(","),
    serverKey: result.key ? JSON.stringify(result.key) : null,
    contact: result.contact,
    terms: result.terms,
    privacy: result.privacy,
    operatorEmail: form.operatorEmail,
    emergencyEmail: form.emergencyEmail,
    report: JSON.stringify(result.checks),
    reason: null,
    appliedAt: new Date(now),
    pledgeAt: new Date(now),
    checkedAt: new Date(now),
    listedAt: null,
    failingSince: null,
    delistedAt: null,
  };
}

export function listEntry(row) {
  return {
    origin: row.origin,
    name: row.name,
    about: row.about ?? null,
    kinds: String(row.kinds ?? "").split(",").filter(Boolean),
    key: row.serverKey ? JSON.parse(row.serverKey) : null,
    contact: row.contact ?? null,
    terms: row.terms ?? null,
    privacy: row.privacy ?? null,
    listed: row.listedAt ? new Date(row.listedAt).getTime() : 0,
    checked: row.checkedAt ? new Date(row.checkedAt).getTime() : 0,
  };
}

export function reportOf(row) {
  try {
    return JSON.parse(row.report ?? "[]");
  } catch {
    return [];
  }
}

export function createListingStore(adapter) {
  const find = (origin) => adapter.findOne({ model: MODEL, where: [{ field: "origin", value: origin }] });
  return {
    find,
    async findByConfirm(token) {
      return adapter.findOne({ model: MODEL, where: [{ field: "confirmHash", value: hash(token) }] });
    },
    async shown() {
      return adapter.findMany({ model: MODEL, where: [{ field: "status", operator: "in", value: SHOWN_STATUSES }], limit: 10000 });
    },
    async all() {
      return adapter.findMany({ model: MODEL, limit: 10000 });
    },
    async save(row) {
      const held = await find(row.origin);
      if (held) return adapter.update({ model: MODEL, where: [{ field: "origin", value: row.origin }], update: row });
      return adapter.create({ model: MODEL, data: row });
    },
    async change(origin, update) {
      return adapter.update({ model: MODEL, where: [{ field: "origin", value: origin }], update });
    },
  };
}

function masked(contact) {
  const [name, domain] = String(contact).split("@");
  return domain ? `${name.slice(0, 1)}***@${domain}` : contact;
}

export function confirmationTarget(row) {
  return contactKind(row.contact)?.kind === "email" ? row.contact : row.operatorEmail;
}

const applyBody = z.object({
  origin: z.string().max(300),
  name: z.string().max(200).optional(),
  about: z.string().max(400).optional(),
  operatorEmail: z.email().max(254),
  emergencyEmail: z.email().max(254),
  terms: z.string().max(500).optional(),
  privacy: z.string().max(500).optional(),
  pledge: z.literal(true),
});

export const commyListing = ({ config, send, checker, onChange = () => {}, now = () => Date.now() }) => {
  const pageUrl = (path, query) => `${config.baseURL}${path}?${new URLSearchParams(query)}`;
  const instance = config.instanceName;

  async function askToConfirm(row, token) {
    await send({
      to: confirmationTarget(row),
      subject: `Confirm the abuse contact for ${row.name}`,
      heading: "Confirm your server's abuse contact",
      body: `${new URL(row.origin).hostname} asked to be listed on ${instance}. Confirm that this address is read by a person who answers abuse reports. The link expires in ${CONFIRM_DAYS} days.`,
      actionLabel: "Confirm the contact",
      url: pageUrl(APPLY_PAGE, { confirm: token }),
    });
  }

  async function askForReview(row) {
    if (!config.listing.reviewEmail) return;
    await send({
      to: config.listing.reviewEmail,
      subject: `Listing request: ${new URL(row.origin).hostname}`,
      heading: `${row.name} asks to be listed`,
      body: `${row.origin} passed every check and its abuse contact is confirmed. Review the application and approve or reject it.`,
      actionLabel: "Review the application",
      url: pageUrl(REVIEW_PAGE, { token: reviewToken(config.secret, row, now()) }),
    });
  }

  async function tellOperator(row, subject, body) {
    await send({ to: row.operatorEmail, subject, heading: subject, body, actionLabel: "Read the listing rules", url: pageUrl(APPLY_PAGE, {}) });
  }

  return {
    id: "commy-listing",
    schema: {
      [MODEL]: {
        fields: {
          origin: { type: "string", required: true, unique: true },
          name: { type: "string", required: true },
          about: { type: "string", required: false },
          kinds: { type: "string", required: true },
          serverKey: { type: "string", required: false },
          contact: { type: "string", required: false },
          terms: { type: "string", required: false },
          privacy: { type: "string", required: false },
          operatorEmail: { type: "string", required: true },
          emergencyEmail: { type: "string", required: true },
          status: { type: "string", required: true, index: true },
          confirmHash: { type: "string", required: false, index: true },
          report: { type: "string", required: false },
          reason: { type: "string", required: false },
          appliedAt: { type: "date", required: true },
          pledgeAt: { type: "date", required: true },
          checkedAt: { type: "date", required: true },
          listedAt: { type: "date", required: false },
          failingSince: { type: "date", required: false },
          delistedAt: { type: "date", required: false },
        },
      },
    },
    rateLimit: [
      { pathMatcher: (path) => path === APPLY_PATH, window: 3600, max: 10 },
      { pathMatcher: (path) => [CONFIRM_PATH, REVIEW_PATH, DECIDE_PATH].includes(path), window: 600, max: 20 },
    ],
    endpoints: {
      applyForListing: createAuthEndpoint(APPLY_PATH, { method: "POST", body: applyBody }, async (ctx) => {
        const store = createListingStore(ctx.context.adapter);
        const result = await checker.check({ origin: ctx.body.origin, terms: ctx.body.terms, privacy: ctx.body.privacy });
        if (!result.passed) return ctx.json({ passed: false, origin: result.origin, checks: result.checks });
        const held = await store.find(result.origin);
        if (held && BUSY_STATUSES.includes(held.status)) {
          throw new APIError("CONFLICT", { message: held.status === STATUS.review ? "This server's application is waiting for review." : "This server is already listed." });
        }
        const token = randomBytes(32).toString("base64url");
        const row = { ...rowFromCheck(result, ctx.body, now()), status: STATUS.confirming, confirmHash: hash(token) };
        await store.save(row);
        await askToConfirm(row, token);
        return ctx.json({ passed: true, origin: result.origin, checks: result.checks, confirmTo: masked(confirmationTarget(row)) });
      }),
      confirmListingContact: createAuthEndpoint(CONFIRM_PATH, { method: "POST", body: z.object({ token: z.string().max(200) }) }, async (ctx) => {
        if (!TOKEN.test(ctx.body.token)) throw new APIError("BAD_REQUEST", { message: "That confirmation link is not valid." });
        const store = createListingStore(ctx.context.adapter);
        const row = await store.findByConfirm(ctx.body.token);
        const fresh = row && row.status === STATUS.confirming && now() - new Date(row.appliedAt).getTime() < CONFIRM_DAYS * DAY_MS;
        if (!fresh) throw new APIError("NOT_FOUND", { message: "That confirmation link has expired or was already used. Apply again to get a new one." });
        await store.change(row.origin, { status: STATUS.review, confirmHash: null });
        await askForReview(row);
        return ctx.json({ ok: true, origin: row.origin });
      }),
      readListingReview: createAuthEndpoint(REVIEW_PATH, { method: "GET", query: z.object({ token: z.string().max(1000) }) }, async (ctx) => {
        const row = await reviewedRow(ctx, ctx.query.token);
        return ctx.json({ application: reviewView(row) });
      }),
      decideListingReview: createAuthEndpoint(
        DECIDE_PATH,
        { method: "POST", body: z.object({ token: z.string().max(1000), decision: z.enum(["approve", "reject"]), note: z.string().max(500).optional() }) },
        async (ctx) => {
          const row = await reviewedRow(ctx, ctx.body.token);
          const store = createListingStore(ctx.context.adapter);
          const note = cleanText(ctx.body.note, 500);
          if (ctx.body.decision === "approve") {
            await store.change(row.origin, { status: STATUS.listed, listedAt: new Date(now()), reason: note });
            await tellOperator(row, `${row.name} is listed`, `${row.origin} is now on ${instance}'s list of servers. It is checked again every month; keep the pledge and the checks passing to stay listed.`);
          } else {
            await store.change(row.origin, { status: STATUS.rejected, reason: note });
            await tellOperator(row, `${row.name} was not listed`, `${row.origin} was not added to ${instance}'s list.${note ? ` The reviewer wrote: ${note}` : ""} You can apply again after fixing it.`);
          }
          await onChange();
          return ctx.json({ ok: true, status: ctx.body.decision === "approve" ? STATUS.listed : STATUS.rejected });
        },
      ),
    },
  };

  async function reviewedRow(ctx, token) {
    const opened = openReviewToken(config.secret, token, now());
    if (!opened) throw new APIError("NOT_FOUND", { message: "That review link has expired or is not valid." });
    const row = await createListingStore(ctx.context.adapter).find(opened.origin);
    if (!row || row.status !== STATUS.review || new Date(row.appliedAt).getTime() !== opened.appliedAt) {
      throw new APIError("NOT_FOUND", { message: "That application was already reviewed or replaced." });
    }
    return row;
  }
};

export function reviewView(row) {
  return {
    origin: row.origin,
    name: row.name,
    about: row.about ?? null,
    kinds: String(row.kinds ?? "").split(",").filter(Boolean),
    contact: row.contact ?? null,
    terms: row.terms ?? null,
    privacy: row.privacy ?? null,
    operatorEmail: row.operatorEmail,
    emergencyEmail: row.emergencyEmail,
    appliedAt: new Date(row.appliedAt).getTime(),
    checks: reportOf(row),
  };
}
