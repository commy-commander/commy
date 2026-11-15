import { createListingStore, reportOf, STATUS } from "./listing.js";

export const RECHECK_DAYS = 30;
export const GRACE_DAYS = 7;
const DAY_MS = 24 * 60 * 60 * 1000;
const TICK_MS = 60 * 60 * 1000;
const FIRST_TICK_MS = 10 * 60 * 1000;
const PER_TICK = 50;

const failedNames = (checks) => checks.filter((check) => !check.ok).map((check) => check.detail).join(" ");

export function isDue(row, now) {
  const since = now - new Date(row.checkedAt).getTime();
  if (row.status === STATUS.listed) return since >= RECHECK_DAYS * DAY_MS;
  if (row.status === STATUS.failing) return since >= DAY_MS;
  return false;
}

export function nextState(row, result, now) {
  const checked = new Date(now);
  const report = JSON.stringify(result.checks);
  if (result.passed) return { update: { status: STATUS.listed, checkedAt: checked, failingSince: null, report }, notice: row.status === STATUS.failing ? "fixed" : null };
  const blocked = result.checks.some((check) => check.id === "address" && !check.ok);
  if (blocked) return { update: { status: STATUS.delisted, checkedAt: checked, delistedAt: checked, reason: failedNames(result.checks), report }, notice: "delisted" };
  if (row.status === STATUS.listed) return { update: { status: STATUS.failing, checkedAt: checked, failingSince: checked, report }, notice: "failing" };
  const failingFor = now - new Date(row.failingSince ?? row.checkedAt).getTime();
  if (failingFor >= GRACE_DAYS * DAY_MS) {
    return { update: { status: STATUS.delisted, checkedAt: checked, delistedAt: checked, reason: failedNames(result.checks), report }, notice: "delisted" };
  }
  return { update: { checkedAt: checked, report }, notice: null };
}

export function noticeEmail(row, notice, instance) {
  const host = new URL(row.origin).hostname;
  const problems = failedNames(reportOf(row));
  if (notice === "failing") {
    return {
      subject: `${host} failed its monthly listing check`,
      body: `${problems} Fix it within ${GRACE_DAYS} days to stay on ${instance}'s list. It is checked again every day until it passes.`,
    };
  }
  if (notice === "delisted") {
    return {
      subject: `${host} was taken off the list`,
      body: `${problems || row.reason || "It no longer meets the listing rules."} The server keeps working, and people are asked to confirm they trust it. Apply again once it is fixed.`,
    };
  }
  return { subject: `${host} passes its listing checks again`, body: `Every check passes again, so ${host} stays on ${instance}'s list.` };
}

export async function recheck(adapter, row, { checker, send, instance, now }) {
  const result = await checker.check({ origin: row.origin, terms: row.terms, privacy: row.privacy });
  const { update, notice } = nextState(row, result, now);
  await createListingStore(adapter).change(row.origin, update);
  if (notice) {
    const after = { ...row, ...update };
    const mail = noticeEmail(after, notice, instance);
    const recipients = [...new Set([row.operatorEmail, row.emergencyEmail, ...(result.contactKind === "email" ? [result.contact] : [])].filter(Boolean))];
    for (const to of recipients) await send({ to, subject: mail.subject, heading: mail.subject, body: mail.body }).catch(() => {});
  }
  return { origin: row.origin, status: update.status ?? row.status, notice };
}

export async function recheckDue(adapter, options) {
  const rows = await createListingStore(adapter).shown();
  const due = rows.filter((row) => isDue(row, options.now)).slice(0, PER_TICK);
  const outcomes = [];
  for (const row of due) outcomes.push(await recheck(adapter, row, options).catch((err) => ({ origin: row.origin, error: err.message })));
  return outcomes;
}

export function startListingRechecks(authContext, { checker, send, instance, onChange, log = console, now = () => Date.now() }) {
  const tick = async () => {
    try {
      const { adapter } = await authContext;
      const outcomes = await recheckDue(adapter, { checker, send, instance, now: now() });
      const changed = outcomes.filter((o) => o.notice);
      for (const o of changed) log.log(`[auth] listing: ${o.origin} is ${o.status}`);
      if (changed.length) await onChange();
    } catch (err) {
      log.warn(`[auth] listing re-checks failed: ${err.message}`);
    }
  };
  const first = setTimeout(tick, FIRST_TICK_MS);
  const repeat = setInterval(tick, TICK_MS);
  first.unref?.();
  repeat.unref?.();
  return () => {
    clearTimeout(first);
    clearInterval(repeat);
  };
}
