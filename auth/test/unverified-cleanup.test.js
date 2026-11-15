import assert from "node:assert/strict";
import { test } from "node:test";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { sealedAtRest } from "../src/at-rest.js";
import { buildAuthOptions } from "../src/auth.js";
import { loadConfig } from "../src/config.js";
import { UNVERIFIED_KEEP_DAYS, sweepAndReport, sweepUnverifiedAccounts } from "../src/unverified-cleanup.js";

const ORIGIN = "http://localhost:3001";
const DAY_MS = 24 * 60 * 60 * 1000;
const PASSWORD = "correct horse battery";

function makeAuth() {
  const config = loadConfig({
    BETTER_AUTH_SECRET: "test-secret-".repeat(4),
    DATABASE_URL: "postgres://unused",
    BETTER_AUTH_URL: ORIGIN,
    SMTP_HOST: "smtp.invalid",
    SMTP_USER: "no-reply@example.test",
    SMTP_PASSWORD: "unused",
  });
  const options = buildAuthOptions(config, { pool: null, send: async () => {} });
  const db = { user: [], session: [], account: [], verification: [], deviceCode: [], linkedDevice: [], deviceRequest: [], invite: [], inviteBackground: [], doorbellRing: [], vaultItem: [], passkey: [], twoFactor: [], directoryListing: [] };
  options.database = sealedAtRest(memoryAdapter(db), options.secret);
  options.rateLimit = { enabled: false };
  return { auth: betterAuth(options), db };
}

async function signUp(auth, email) {
  const res = await auth.handler(
    new Request(`${ORIGIN}/api/auth/sign-up/email`, {
      method: "POST",
      headers: { origin: ORIGIN, "content-type": "application/json" },
      body: JSON.stringify({ email, password: PASSWORD, name: "Someone" }),
    }),
  );
  assert.equal(res.status, 200);
}

const ageBy = (db, index, days) => {
  db.user[index].createdAt = new Date(Date.now() - days * DAY_MS);
};

test("accounts that never verified their email are removed after a week, others stay", async () => {
  const { auth, db } = makeAuth();
  await signUp(auth, "stale@example.com");
  await signUp(auth, "fresh@example.com");
  await signUp(auth, "verified@example.com");
  await signUp(auth, "linked@example.com");
  ageBy(db, 0, UNVERIFIED_KEEP_DAYS + 1);
  ageBy(db, 1, UNVERIFIED_KEEP_DAYS - 1);
  ageBy(db, 2, UNVERIFIED_KEEP_DAYS + 1);
  db.user[2].emailVerified = true;
  ageBy(db, 3, UNVERIFIED_KEEP_DAYS + 1);
  const linkedId = db.user[3].id;
  db.account.push({ id: "twitch-row", userId: linkedId, providerId: "twitch", accountId: "t-1", createdAt: new Date(), updatedAt: new Date() });

  const staleId = db.user[0].id;
  const removed = await sweepUnverifiedAccounts(await auth.$context);
  assert.equal(removed, 1);
  assert.equal(db.user.length, 3);
  assert.ok(!db.user.some((user) => user.id === staleId));
  assert.ok(!db.account.some((account) => account.userId === staleId));
  assert.ok(db.user.some((user) => user.id === linkedId));

  assert.equal(await sweepUnverifiedAccounts(await auth.$context), 0);
});

test("each sweep logs what it removed and a failure only warns", async () => {
  const lines = [];
  const log = { log: (line) => lines.push(line), warn: (line) => lines.push(line) };
  const { auth, db } = makeAuth();
  await signUp(auth, "stale@example.com");
  ageBy(db, 0, UNVERIFIED_KEEP_DAYS + 1);
  assert.equal(await sweepAndReport(auth.$context, { log }), 1);
  assert.deepEqual(lines, ["[auth] removed 1 accounts whose email was never verified"]);

  const broken = {
    adapter: {
      findMany: async () => {
        throw new Error("database away");
      },
    },
    internalAdapter: {},
  };
  assert.equal(await sweepAndReport(broken, { log }), 0);
  assert.equal(lines.at(-1), "[auth] removing never-verified accounts failed: database away");
});
