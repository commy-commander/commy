import { appendFile, readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

export const NIGHTLY = "nightly";
export const KEEP_NIGHTLIES = 7;
export const RELEASES = "https://releases.commy.chat";
const CACHE_WARMING_DAYS = [1, 4];
const NIGHTLY_SHAPE = /^(\d+)\.(\d+)\.(\d+)-nightly\.(\d{8})\.(\d+)$/;

export function versionCore(version) {
  const match = /^v?(\d+)\.(\d+)\.(\d+)(?:[-+].*)?$/.exec(String(version ?? "").trim());
  return match ? match.slice(1, 4).map(Number) : null;
}

function compareCores(a, b) {
  for (let i = 0; i < 3; i++) {
    if (a[i] !== b[i]) return a[i] - b[i];
  }
  return 0;
}

function utcDay(date) {
  return date.toISOString().slice(0, 10).replace(/-/g, "");
}

export function nightlyVersion({ appVersion, stableVersion, date, run }) {
  const app = versionCore(appVersion);
  const stable = versionCore(stableVersion);
  const afterStable = stable ? [stable[0], stable[1], stable[2] + 1] : null;
  const candidates = [app, afterStable].filter(Boolean);
  if (candidates.length === 0) throw new Error(`no version to build a nightly from (app ${appVersion}, stable ${stableVersion})`);
  const base = candidates.reduce((best, next) => (compareCores(next, best) > 0 ? next : best));
  if (!/^\d+$/.test(String(run)) || String(run).startsWith("0")) throw new Error(`not a run number: ${run}`);
  return `${base.join(".")}-${NIGHTLY}.${utcDay(date)}.${run}`;
}

export function isNightlyVersion(version) {
  return NIGHTLY_SHAPE.test(version ?? "");
}

export function nightlyOrder(a, b) {
  const [, , , , dayA, runA] = NIGHTLY_SHAPE.exec(a);
  const [, , , , dayB, runB] = NIGHTLY_SHAPE.exec(b);
  return dayA === dayB ? Number(runA) - Number(runB) : dayA < dayB ? -1 : 1;
}

export function nightliesToPrune(versions, keep = KEEP_NIGHTLIES) {
  const newestFirst = versions.filter(isNightlyVersion).sort(nightlyOrder).reverse();
  return newestFirst.slice(Math.max(keep, 1));
}

export function buildPlan({ event, ref, enabled, asked, sha, lastCommit, date }) {
  const onMain = ref === "refs/heads/main";
  if (asked) {
    if (!onMain || !enabled) throw new Error("nightly builds run only from main, with the NIGHTLY_BUILDS variable set to true");
    return { build: true, nightly: true, reason: "a nightly was asked for" };
  }
  if (event !== "schedule") return { build: true, nightly: false, reason: `a ${event} build` };
  if (enabled && onMain) {
    if (lastCommit && lastCommit === sha) return { build: false, nightly: false, reason: "main has not changed since the last nightly" };
    return { build: true, nightly: true, reason: "main changed since the last nightly" };
  }
  const warm = CACHE_WARMING_DAYS.includes(date.getUTCDay());
  return { build: warm, nightly: false, reason: warm ? "keeping the build caches warm" : "nightly builds are off" };
}

async function fetchManifest(url) {
  try {
    const response = await fetch(url, { headers: { "cache-control": "no-cache" } });
    return response.ok ? await response.json() : null;
  } catch {
    return null;
  }
}

export async function planFromEnv({ env = process.env, date = new Date(), fetchJson = fetchManifest, readText = (path) => readFile(path, "utf8") } = {}) {
  const asked = env.NIGHTLY_INPUT === "true";
  const enabled = env.NIGHTLY_BUILDS === "true";
  const wantsNightly = asked || (env.GITHUB_EVENT_NAME === "schedule" && enabled && env.GITHUB_REF === "refs/heads/main");
  const last = wantsNightly && !asked ? await fetchJson(`${RELEASES}/${NIGHTLY}/latest.json`) : null;
  const plan = buildPlan({
    event: env.GITHUB_EVENT_NAME,
    ref: env.GITHUB_REF,
    enabled,
    asked,
    sha: env.GITHUB_SHA,
    lastCommit: last?.commit ?? null,
    date,
  });
  if (!plan.nightly) return { ...plan, version: "" };
  const stable = await fetchJson(`${RELEASES}/stable/latest.json`);
  const config = JSON.parse(await readText(env.TAURI_CONFIG ?? "apps/desktop/src-tauri/tauri.conf.json"));
  const version = nightlyVersion({ appVersion: config.version, stableVersion: stable?.version, date, run: env.GITHUB_RUN_NUMBER });
  return { ...plan, version };
}

const isMain = process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href;
if (isMain) {
  planFromEnv()
    .then(async (plan) => {
      console.log(`${plan.build ? "Building" : "Not building"}: ${plan.reason}${plan.version ? ` (${plan.version})` : ""}`);
      if (process.env.GITHUB_OUTPUT) await appendFile(process.env.GITHUB_OUTPUT, `build=${plan.build}\nnightly=${plan.version}\n`);
    })
    .catch((error) => {
      console.error(error.message);
      process.exit(1);
    });
}
