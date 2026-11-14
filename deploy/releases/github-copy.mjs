import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";
import { PUBLIC_BASE, contentType, parseTag } from "./publish.mjs";

export const GITHUB_API = "https://api.github.com";
export const GITHUB_UPLOADS = "https://uploads.github.com";
export const DEFAULT_GITHUB_REPO = "commy-commander/commy";
export const COPIED_FILES = ["latest.json", "SHA256SUMS.txt", "SHA256SUMS.txt.bundle"];

export function isStableTag(tag) {
  try {
    return parseTag(tag).channel === "stable";
  } catch {
    return false;
  }
}

export function stableReleases(forgeReleases) {
  return forgeReleases.filter((release) => !release.draft && !release.prerelease && isStableTag(release.tag_name));
}

export function compareTags(a, b) {
  const parts = (tag) => parseTag(tag).version.split(".").map(Number);
  const [left, right] = [parts(a), parts(b)];
  for (let index = 0; index < 3; index += 1) {
    if (left[index] !== right[index]) return left[index] - right[index];
  }
  return 0;
}

export function removeRepeatedHeadings(body) {
  const seen = new Set();
  const kept = [];
  let skipBlank = false;
  for (const line of (body ?? "").split("\n")) {
    if (skipBlank && line.trim() === "") {
      skipBlank = false;
      continue;
    }
    skipBlank = false;
    if (/^#{1,6} /.test(line)) {
      if (seen.has(line)) {
        skipBlank = true;
        continue;
      }
      seen.add(line);
    }
    kept.push(line);
  }
  return kept.join("\n");
}

export function copyPlan({ forge, github }) {
  const releases = stableReleases(forge).sort((a, b) => compareTags(a.tag_name, b.tag_name));
  if (releases.length === 0) return [];
  const newest = releases[releases.length - 1].tag_name;
  const steps = [];
  for (const release of releases) {
    const tag = release.tag_name;
    const body = removeRepeatedHeadings(release.body);
    const copy = github.get(tag);
    if (!copy) {
      steps.push({ tag, action: "create", name: release.name || `Commy ${tag}`, body, latest: tag === newest, files: COPIED_FILES });
      continue;
    }
    if (copy.body !== body) steps.push({ tag, action: "update", id: copy.id, body });
    const missing = COPIED_FILES.filter((name) => !copy.assets.includes(name));
    if (missing.length) steps.push({ tag, action: "upload", id: copy.id, files: missing });
  }
  return steps;
}

export function tagsToPush(steps) {
  return steps.filter((step) => step.action === "create").map((step) => step.tag);
}

export function gitPushCommand({ repo, token, tags }) {
  const basic = Buffer.from(`x-access-token:${token}`).toString("base64");
  return {
    args: ["push", `https://github.com/${repo}.git`, "HEAD:refs/heads/main", ...tags.map((tag) => `refs/tags/${tag}`)],
    env: {
      GIT_CONFIG_COUNT: "1",
      GIT_CONFIG_KEY_0: "http.https://github.com/.extraheader",
      GIT_CONFIG_VALUE_0: `AUTHORIZATION: basic ${basic}`,
    },
  };
}

async function expectOk(response, what) {
  if (response.ok) return response;
  throw new Error(`${what} failed: ${response.status} ${await response.text()}`);
}

function githubHeaders(token, extra = {}) {
  return {
    ...(token ? { Authorization: `Bearer ${token}` } : {}),
    Accept: "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
    "User-Agent": "commy-release-copy",
    ...extra,
  };
}

export async function readForgeReleases({ forgeUrl, forgeRepo, forgeToken, fetch }) {
  const headers = forgeToken ? { Authorization: `token ${forgeToken}` } : {};
  const response = await fetch(`${forgeUrl}/api/v1/repos/${forgeRepo}/releases?limit=50`, { headers });
  return (await expectOk(response, "reading forge releases")).json();
}

export async function readGithubCopies({ tags, repo, token, fetch }) {
  const copies = new Map();
  for (const tag of tags) {
    const response = await fetch(`${GITHUB_API}/repos/${repo}/releases/tags/${encodeURIComponent(tag)}`, {
      headers: githubHeaders(token),
    });
    if (response.status === 404) continue;
    const release = await (await expectOk(response, `reading GitHub release ${tag}`)).json();
    copies.set(tag, { id: release.id, body: release.body ?? "", assets: release.assets.map((asset) => asset.name) });
  }
  return copies;
}

async function uploadFiles({ tag, id, files, repo, token, fetch, log }) {
  for (const name of files) {
    const source = await fetch(`${PUBLIC_BASE}/${tag}/${name}`);
    if (source.status === 404) {
      log(`${tag}: ${name} is not on releases.commy.chat, skipped`);
      continue;
    }
    const data = Buffer.from(await (await expectOk(source, `downloading ${tag}/${name}`)).arrayBuffer());
    const upload = await fetch(`${GITHUB_UPLOADS}/repos/${repo}/releases/${id}/assets?name=${encodeURIComponent(name)}`, {
      method: "POST",
      headers: githubHeaders(token, { "Content-Type": contentType(name) }),
      body: data,
    });
    await expectOk(upload, `uploading ${tag}/${name}`);
    log(`${tag}: uploaded ${name}`);
  }
}

async function applyStep(step, { repo, token, fetch, log }) {
  if (step.action === "create") {
    const response = await fetch(`${GITHUB_API}/repos/${repo}/releases`, {
      method: "POST",
      headers: githubHeaders(token, { "Content-Type": "application/json" }),
      body: JSON.stringify({
        tag_name: step.tag,
        name: step.name,
        body: step.body,
        draft: false,
        prerelease: false,
        make_latest: String(step.latest),
      }),
    });
    const release = await (await expectOk(response, `creating GitHub release ${step.tag}`)).json();
    log(`${step.tag}: created ${release.html_url}`);
    await uploadFiles({ tag: step.tag, id: release.id, files: step.files, repo, token, fetch, log });
  } else if (step.action === "update") {
    const response = await fetch(`${GITHUB_API}/repos/${repo}/releases/${step.id}`, {
      method: "PATCH",
      headers: githubHeaders(token, { "Content-Type": "application/json" }),
      body: JSON.stringify({ body: step.body }),
    });
    await expectOk(response, `updating notes of ${step.tag}`);
    log(`${step.tag}: notes updated`);
  } else if (step.action === "upload") {
    await uploadFiles({ tag: step.tag, id: step.id, files: step.files, repo, token, fetch, log });
  }
}

export async function copyReleases({
  env = process.env,
  fetch = globalThis.fetch,
  git = (args, extraEnv) => execFileSync("git", args, { stdio: "inherit", env: { ...process.env, ...extraEnv } }),
  dryRun = false,
  log = console.log,
}) {
  const token = env.MIRROR_GITHUB_TOKEN;
  const repo = env.GITHUB_COPY_REPO || DEFAULT_GITHUB_REPO;
  if (!token && !dryRun) {
    log("warning: MIRROR_GITHUB_TOKEN is not set, nothing copied to GitHub");
    return [];
  }
  const forge = await readForgeReleases({
    forgeUrl: env.GITHUB_SERVER_URL || "https://git.commy.chat",
    forgeRepo: env.GITHUB_REPOSITORY || "commy/commy",
    forgeToken: env.FORGE_TOKEN,
    fetch,
  });
  const tags = stableReleases(forge).map((release) => release.tag_name);
  const github = await readGithubCopies({ tags, repo, token, fetch });
  const steps = copyPlan({ forge, github });
  if (steps.length === 0) {
    log("GitHub already has every release");
    return steps;
  }
  for (const step of steps) log(`${step.tag}: ${step.action}`);
  if (dryRun) return steps;
  const newTags = tagsToPush(steps);
  if (newTags.length) {
    const push = gitPushCommand({ repo, token, tags: newTags });
    git(push.args, push.env);
  }
  for (const step of steps) await applyStep(step, { repo, token, fetch, log });
  return steps;
}

const isMain = process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href;
if (isMain) {
  copyReleases({ dryRun: process.argv.includes("--dry-run") }).catch((error) => {
    console.error(error.message);
    process.exit(1);
  });
}
