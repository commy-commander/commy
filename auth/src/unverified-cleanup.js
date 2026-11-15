import { eraseAccount } from "./account-deletion.js";

export const UNVERIFIED_KEEP_DAYS = 7;
const DAY_MS = 24 * 60 * 60 * 1000;
const FIRST_SWEEP_AFTER_MS = 15 * 60 * 1000;
const SWEEP_EVERY_MS = 6 * 60 * 60 * 1000;
const SWEEP_BATCH = 200;
const PASSWORD_PROVIDER = "credential";

async function signsInOnlyWithPassword(adapter, userId) {
  const accounts = await adapter.findMany({ model: "account", where: [{ field: "userId", value: userId }] });
  return accounts.length > 0 && accounts.every((account) => account.providerId === PASSWORD_PROVIDER);
}

export async function sweepUnverifiedAccounts({ adapter, internalAdapter }, now = Date.now()) {
  const cutoff = new Date(now - UNVERIFIED_KEEP_DAYS * DAY_MS);
  const stale = await adapter.findMany({
    model: "user",
    where: [
      { field: "emailVerified", value: false },
      { field: "createdAt", operator: "lt", value: cutoff },
    ],
    limit: SWEEP_BATCH,
  });
  let removed = 0;
  for (const user of stale) {
    if (!(await signsInOnlyWithPassword(adapter, user.id))) continue;
    await eraseAccount(adapter, internalAdapter, user.id);
    removed++;
  }
  return removed;
}

export function sweepAndReport(authContext, { log = console } = {}) {
  return Promise.resolve(authContext)
    .then((context) => sweepUnverifiedAccounts(context))
    .then((removed) => {
      if (removed) log.log(`[auth] removed ${removed} accounts whose email was never verified`);
      return removed;
    })
    .catch((err) => {
      log.warn(`[auth] removing never-verified accounts failed: ${err.message}`);
      return 0;
    });
}

export function startUnverifiedSweeps(authContext, { log = console } = {}) {
  const sweep = () => sweepAndReport(authContext, { log });
  const first = setTimeout(sweep, FIRST_SWEEP_AFTER_MS);
  const repeat = setInterval(sweep, SWEEP_EVERY_MS);
  first.unref();
  repeat.unref();
  return () => {
    clearTimeout(first);
    clearInterval(repeat);
  };
}
