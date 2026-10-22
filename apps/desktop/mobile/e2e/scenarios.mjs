import { sleep } from "./adb.mjs";
import { connect } from "./cdp.mjs";
import { PATTERNS, countActiveNotifications, crashes, find, parseEpochLog, registeredTags, since, wakes } from "./logs.mjs";

const START_TIMEOUT_MS = 90_000;
const CRASH_WATCH_MS = 10_000;
const REGISTER_TIMEOUT_MS = 90_000;
const WAKE_TIMEOUT_MS = 180_000;
const BURST_SPACING_MS = 5_000;
const BURST_SIZE = 3;
const BADGE_SETTLE_MS = 5_000;
const DELIVERY_TIMEOUT_MS = 90_000;
const POLL_MS = 1_000;

async function logSince(ctx, at) {
  return since(parseEpochLog(await ctx.device.readLog()), at);
}

async function waitForLog(ctx, pattern, at, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const hit = find(await logSince(ctx, at), pattern);
    if (hit || Date.now() > deadline) return hit;
    await sleep(POLL_MS);
  }
}

async function waitFor(check, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = await check();
    if (value || Date.now() > deadline) return value;
    await sleep(POLL_MS);
  }
}

export function nonce(label) {
  return `e2e ${label} ${Date.now().toString(36)}`;
}

async function openTablet(ctx) {
  const pid = await ctx.device.pid(ctx.pkg);
  if (!pid) throw new Error("Commy is not running on the device");
  const unforward = await ctx.device.forwardWebview(pid, ctx.tabletDebugPort);
  try {
    const app = await waitFor(() => connect(ctx.tabletDebugPort).catch(() => null), 30_000);
    if (!app) throw new Error("could not reach Commy's webview on the device");
    return { app, close: async () => { app.close(); await unforward(); } };
  } catch (e) {
    await unforward();
    throw e;
  }
}

async function withTablet(ctx, use) {
  const tablet = await openTablet(ctx);
  try {
    return await use(tablet.app);
  } finally {
    await tablet.close();
  }
}

async function bringUp(ctx) {
  const at = await ctx.device.deviceNowMs();
  await ctx.device.launch(ctx.pkg);
  const already = await ctx.device.pid(ctx.pkg);
  const connected = await waitForLog(ctx, PATTERNS.webviewConnected, at, already ? 5_000 : START_TIMEOUT_MS);
  return { at, connected: Boolean(connected) || Boolean(already) };
}

async function freshStart(ctx) {
  await ctx.device.shell(`am force-stop ${ctx.pkg}`);
  const at = await ctx.device.deviceNowMs();
  await ctx.device.launch(ctx.pkg);
  const connected = await waitForLog(ctx, PATTERNS.webviewConnected, at, START_TIMEOUT_MS);
  return { at, connected: Boolean(connected) };
}

async function closeTablet(ctx) {
  await ctx.device.closeLikeSwipe(ctx.pkg);
  return waitFor(async () => !(await ctx.device.pid(ctx.pkg)), 10_000);
}

export async function launch(ctx) {
  const details = {};
  if (ctx.apk) details.install = await ctx.device.install(ctx.apk, ctx.pkg);
  const { at } = await freshStart(ctx);
  const connected = find(await logSince(ctx, at), PATTERNS.webviewConnected);
  await sleep(CRASH_WATCH_MS);
  const entries = await logSince(ctx, at);
  const pid = await ctx.device.pid(ctx.pkg);
  const crashed = crashes(entries, pid ? [pid] : []);
  details.startMs = connected ? connected.at - at : null;
  details.firebase = Boolean(find(entries, PATTERNS.firebaseReady));
  details.running = Boolean(pid);
  details.crashes = crashed.map(e => e.text);
  return { ok: Boolean(connected) && Boolean(pid) && crashed.length === 0, details };
}

export async function accounts(ctx) {
  const tablet = await withTablet(ctx, async app => ({
    account: await app.invoke("account_status"),
    user: await app.invoke("get_user"),
    list: await app.invoke("entry_list_status"),
  }));
  const bench = { account: await ctx.bench.invoke("account_status"), user: await ctx.bench.invoke("get_user") };
  ctx.tabletUser = tablet.user?.id ?? null;
  ctx.benchUser = bench.user?.id ?? null;
  const details = {
    tabletSignedIn: Boolean(tablet.account?.signed_in),
    benchSignedIn: Boolean(bench.account?.signed_in),
    sameServer: tablet.account?.server === bench.account?.server,
    server: tablet.account?.server ?? null,
    tabletLinked: Boolean(tablet.account?.device_linked),
    differentPeople: Boolean(ctx.tabletUser && ctx.benchUser && ctx.tabletUser !== ctx.benchUser),
    tabletServerList: tablet.list?.state ?? null,
  };
  const ok = details.tabletSignedIn && details.benchSignedIn && details.sameServer && details.tabletLinked && details.differentPeople;
  return { ok, details };
}

async function needPeople(ctx) {
  if (!ctx.tabletUser || !ctx.benchUser) {
    const result = await accounts(ctx);
    if (!result.ok) throw new Error(`accounts are not ready: ${JSON.stringify(result.details)}`);
  }
}

async function sendFromBench(ctx, label) {
  const text = nonce(label);
  ctx.sent.push(text);
  await ctx.bench.invoke("send_dm", { userId: ctx.tabletUser, content: text, replyTo: null });
  return text;
}

export async function wake(ctx) {
  await needPeople(ctx);
  const details = {};
  const up = await freshStart(ctx);
  if (!up.connected) return { ok: false, details: { reason: "Commy did not start on the device" } };
  await sendFromBench(ctx, "prime");
  const registered = await waitFor(async () => {
    const entries = await logSince(ctx, up.at);
    if (find(entries, PATTERNS.wakeOff)) return "off";
    return registeredTags(entries);
  }, REGISTER_TIMEOUT_MS);
  if (registered === "off") return { ok: false, details: { reason: "the sync service has no push key, so wake-ups are off" } };
  details.registeredTags = registered;
  if (!registered) return { ok: false, details: { ...details, reason: "the device never registered for wake-ups" } };
  if (!(await closeTablet(ctx))) return { ok: false, details: { ...details, reason: "could not close Commy on the device" } };
  await sleep(ctx.pushGapMs);
  const at = await ctx.device.deviceNowMs();
  await sendFromBench(ctx, "wake");
  const woke = await waitForLog(ctx, PATTERNS.wakeReceived, at, WAKE_TIMEOUT_MS);
  if (!woke) return { ok: false, details: { ...details, reason: `no wake-up push within ${WAKE_TIMEOUT_MS / 1000}s` } };
  const [wake] = wakes([woke]);
  details.latencyMs = woke.at - at;
  details.foreground = wake.foreground;
  details.notificationsAllowed = wake.notificationsAllowed;
  await sleep(2_000);
  details.notifications = countActiveNotifications(await ctx.device.notifications(), ctx.pkg);
  return { ok: !wake.foreground && wake.notificationsAllowed && details.notifications > 0, details };
}

export async function burst(ctx) {
  await needPeople(ctx);
  await closeTablet(ctx);
  await sleep(ctx.pushGapMs + 2_000);
  const at = await ctx.device.deviceNowMs();
  for (let i = 0; i < BURST_SIZE; i++) {
    await sendFromBench(ctx, `burst ${i + 1}`);
    if (i < BURST_SIZE - 1) await sleep(BURST_SPACING_MS);
  }
  await sleep(ctx.pushGapMs + 10_000);
  const got = wakes(await logSince(ctx, at)).map(w => w.at - at);
  const details = { wakesAtMs: got, expected: "one right away and one when the push gap ends" };
  return { ok: got.length >= 2, details };
}

export async function badge(ctx) {
  const before = countActiveNotifications(await ctx.device.notifications(), ctx.pkg);
  const up = await bringUp(ctx);
  await sleep(BADGE_SETTLE_MS);
  const after = countActiveNotifications(await ctx.device.notifications(), ctx.pkg);
  return { ok: up.connected && after === 0, details: { before, after } };
}

export async function delivery(ctx) {
  await needPeople(ctx);
  const up = await bringUp(ctx);
  if (!up.connected) return { ok: false, details: { reason: "Commy did not start on the device" } };
  if (ctx.sent.length === 0) await sendFromBench(ctx, "delivery");
  const pending = await waitFor(async () => {
    const messages = (await ctx.bench.invoke("get_dm_messages", { userId: ctx.tabletUser })) ?? [];
    const sent = messages.filter(m => m.outgoing && ctx.sent.includes(m.content));
    const waiting = sent.filter(m => m.pending).length;
    return sent.length > 0 && waiting === 0 ? { delivered: sent.length } : null;
  }, DELIVERY_TIMEOUT_MS);
  return { ok: Boolean(pending), details: pending ?? { reason: "some test DMs still show as waiting to send" } };
}

export const SCENARIOS = { launch, accounts, wake, burst, badge, delivery };
