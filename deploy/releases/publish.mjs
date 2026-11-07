import { createHash, createHmac } from "node:crypto";
import { readdir, readFile, stat } from "node:fs/promises";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

export const PUBLIC_BASE = "https://releases.commy.chat";
export const DOWNLOAD_PAGE = "https://commy.chat/download";
export const DEFAULT_BUCKET = "commy-releases";
export const MANIFEST = "latest.json";
export const INDEX = "release.json";
export const CHANNEL_COPIES = ["commy-soundboard.streamDeckPlugin"];
export const IMMUTABLE = "public, max-age=31536000, immutable";
export const POINTER = "public, max-age=60, must-revalidate";

const CONTENT_TYPES = [
  [".exe", "application/vnd.microsoft.portable-executable"],
  [".zip", "application/zip"],
  [".dmg", "application/x-apple-diskimage"],
  [".tar.gz", "application/gzip"],
  [".AppImage", "application/octet-stream"],
  [".deb", "application/vnd.debian.binary-package"],
  [".json", "application/json"],
  [".txt", "text/plain; charset=utf-8"],
  [".pem", "application/x-pem-file"],
];

export function parseTag(tag) {
  const match = /^v(\d+\.\d+\.\d+)(?:-([0-9A-Za-z.-]+))?$/.exec(tag ?? "");
  if (!match) throw new Error(`not a release tag: ${tag}`);
  const pre = match[2] ?? "";
  const version = pre ? `${match[1]}-${pre}` : match[1];
  const isTestTag = pre.split(".").some((part) => part.startsWith("test"));
  const channel = isTestTag ? null : pre ? "rc" : "stable";
  return { tag, version, prerelease: pre !== "", channel };
}

export function contentType(name) {
  const found = CONTENT_TYPES.find(([suffix]) => name.endsWith(suffix));
  return found ? found[1] : "application/octet-stream";
}

export function publicUrl(key) {
  return `${PUBLIC_BASE}/${key}`;
}

export function releaseIndex({ tag, publishedAt, files }) {
  const { prerelease } = parseTag(tag);
  return {
    tag_name: tag,
    name: `Commy ${tag}`,
    html_url: DOWNLOAD_PAGE,
    published_at: publishedAt,
    prerelease,
    assets: files
      .filter((file) => file.name !== INDEX)
      .map((file) => ({ name: file.name, browser_download_url: publicUrl(`${tag}/${file.name}`), size: file.size })),
  };
}

export function manifestProblems(manifest, tag, names) {
  const { version } = parseTag(tag);
  const problems = [];
  if (manifest?.version !== version) problems.push(`${MANIFEST} says version ${manifest?.version}, the tag says ${version}`);
  const platforms = Object.entries(manifest?.platforms ?? {});
  if (platforms.length === 0) problems.push(`${MANIFEST} lists no platforms`);
  const prefix = publicUrl(`${tag}/`);
  for (const [platform, entry] of platforms) {
    const url = entry?.url ?? "";
    if (!url.startsWith(prefix)) {
      problems.push(`${platform} points at ${url}, outside ${prefix}`);
    } else if (!names.includes(url.slice(prefix.length))) {
      problems.push(`${platform} points at ${url}, which is not in this release`);
    }
    if (!entry?.signature) problems.push(`${platform} has no signature`);
  }
  return problems;
}

export function uploadPlan({ tag, files, publishedAt }) {
  const { channel } = parseTag(tag);
  const names = files.map((file) => file.name);
  const versioned = files.map((file) => ({ key: `${tag}/${file.name}`, file: file.name, cacheControl: IMMUTABLE, contentType: contentType(file.name) }));
  if (!channel) return versioned;
  const copies = CHANNEL_COPIES.filter((name) => names.includes(name)).map((name) => ({
    key: `${channel}/${name}`,
    file: name,
    cacheControl: POINTER,
    contentType: contentType(name),
  }));
  const index = {
    key: `${channel}/${INDEX}`,
    body: JSON.stringify(releaseIndex({ tag, publishedAt, files }), null, 2),
    cacheControl: POINTER,
    contentType: contentType(INDEX),
  };
  const manifest = names.includes(MANIFEST)
    ? [{ key: `${channel}/${MANIFEST}`, file: MANIFEST, cacheControl: POINTER, contentType: contentType(MANIFEST) }]
    : [];
  return [...versioned, ...copies, index, ...manifest];
}

const CHANNELS = ["stable", "rc"];

export function filesPlan(folder, files) {
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(folder ?? "") || CHANNELS.includes(folder)) throw new Error(`not a file folder: ${folder}`);
  return files.map((file) => ({ key: `${folder}/${file.name}`, file: file.name, cacheControl: IMMUTABLE, contentType: contentType(file.name) }));
}

const sha256 = (data) => createHash("sha256").update(data).digest("hex");
const hmac = (key, data) => createHmac("sha256", key).update(data).digest();

export function encodeKey(key) {
  return key
    .split("/")
    .map((segment) => encodeURIComponent(segment).replace(/[!'()*]/g, (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`))
    .join("/");
}

function amzDate(now) {
  return now.toISOString().replace(/[-:]/g, "").replace(/\.\d{3}/, "");
}

export function signRequest({ method, url, headers = {}, body = "", accessKeyId, secretAccessKey, region = "auto", now = new Date() }) {
  const target = new URL(url);
  const stamp = amzDate(now);
  const day = stamp.slice(0, 8);
  const payloadHash = sha256(body);
  const all = { ...headers, host: target.host, "x-amz-content-sha256": payloadHash, "x-amz-date": stamp };
  const lower = Object.fromEntries(Object.entries(all).map(([name, value]) => [name.toLowerCase(), String(value).trim()]));
  const signedHeaders = Object.keys(lower).sort();
  const canonicalQuery = [...target.searchParams.entries()]
    .map(([name, value]) => `${encodeURIComponent(name)}=${encodeURIComponent(value)}`)
    .sort()
    .join("&");
  const canonicalRequest = [
    method,
    target.pathname,
    canonicalQuery,
    signedHeaders.map((name) => `${name}:${lower[name]}\n`).join(""),
    signedHeaders.join(";"),
    payloadHash,
  ].join("\n");
  const scope = `${day}/${region}/s3/aws4_request`;
  const toSign = ["AWS4-HMAC-SHA256", stamp, scope, sha256(canonicalRequest)].join("\n");
  const signingKey = hmac(hmac(hmac(hmac(`AWS4${secretAccessKey}`, day), region), "s3"), "aws4_request");
  const signature = createHmac("sha256", signingKey).update(toSign).digest("hex");
  return {
    ...all,
    authorization: `AWS4-HMAC-SHA256 Credential=${accessKeyId}/${scope}, SignedHeaders=${signedHeaders.join(";")}, Signature=${signature}`,
  };
}

async function readDist(dir) {
  const names = (await readdir(dir)).sort();
  const files = [];
  for (const name of names) {
    const info = await stat(join(dir, name));
    if (info.isFile()) files.push({ name, size: info.size });
  }
  return files;
}

export function accountIdProblem(value) {
  const id = value ?? "";
  if (/^[0-9a-f]{32}$/.test(id)) return null;
  const shape = id.trim() !== id ? "has spaces or line breaks around it" : /^[0-9a-f]+$/i.test(id) ? `is ${id.length} hex characters` : `is ${id.length} characters, not all hex`;
  return `R2_ACCOUNT_ID should be the 32 lowercase hex characters of the Cloudflare account ID, but it ${shape}`;
}

function credentialsFrom(env) {
  const missing = ["R2_ACCOUNT_ID", "R2_ACCESS_KEY_ID", "R2_SECRET_ACCESS_KEY"].filter((name) => !env[name]);
  if (missing.length) throw new Error(`missing ${missing.join(", ")}`);
  const problem = accountIdProblem(env.R2_ACCOUNT_ID);
  if (problem) throw new Error(problem);
  return {
    endpoint: `https://${env.R2_ACCOUNT_ID}.r2.cloudflarestorage.com`,
    bucket: env.R2_BUCKET || DEFAULT_BUCKET,
    accessKeyId: env.R2_ACCESS_KEY_ID,
    secretAccessKey: env.R2_SECRET_ACCESS_KEY,
  };
}

async function upload(credentials, step, body) {
  const url = `${credentials.endpoint}/${credentials.bucket}/${encodeKey(step.key)}`;
  const headers = signRequest({
    method: "PUT",
    url,
    body,
    headers: { "content-type": step.contentType, "cache-control": step.cacheControl },
    accessKeyId: credentials.accessKeyId,
    secretAccessKey: credentials.secretAccessKey,
  });
  const response = await fetch(url, { method: "PUT", headers, body }).catch((error) => {
    throw new Error(`${step.key}: could not reach R2 (${error.cause?.code ?? error.cause?.message ?? error.message})`);
  });
  if (!response.ok) throw new Error(`${step.key}: R2 answered ${response.status} ${await response.text()}`);
}

const UPLOAD_ATTEMPTS = 3;
const RETRY_DELAY_MS = 2000;

async function uploadWithRetries(credentials, step, body, log) {
  for (let attempt = 1; ; attempt++) {
    try {
      return await upload(credentials, step, body);
    } catch (error) {
      if (attempt >= UPLOAD_ATTEMPTS || / answered 4\d\d /.test(error.message)) throw error;
      log(`${error.message}, trying again`);
      await new Promise((resolve) => setTimeout(resolve, RETRY_DELAY_MS * attempt));
    }
  }
}

async function uploadAll(plan, dir, { env, dryRun, log }) {
  const credentials = dryRun ? null : credentialsFrom(env);
  for (const step of plan) {
    log(`${dryRun ? "would upload" : "uploading"} ${step.key}`);
    if (!dryRun) await uploadWithRetries(credentials, step, step.body ?? (await readFile(join(dir, step.file))), log);
  }
}

export async function publishFiles({ folder, dir, env = process.env, dryRun = false, log = console.log }) {
  const files = await readDist(dir);
  if (files.length === 0) throw new Error(`${dir} has no files`);
  const plan = filesPlan(folder, files);
  await uploadAll(plan, dir, { env, dryRun, log });
  log(`${files.length} files are under ${publicUrl(`${folder}/`)}`);
  return plan;
}

export async function publish({ tag, dir, env = process.env, dryRun = false, log = console.log }) {
  const files = await readDist(dir);
  if (files.length === 0) throw new Error(`${dir} has no files`);
  const names = files.map((file) => file.name);
  if (names.includes(MANIFEST)) {
    const problems = manifestProblems(JSON.parse(await readFile(join(dir, MANIFEST), "utf8")), tag, names);
    if (problems.length) throw new Error(`refusing to publish:\n${problems.join("\n")}`);
  } else {
    log(`warning: no ${MANIFEST}, installed apps will not update to ${tag}`);
  }
  const plan = uploadPlan({ tag, files, publishedAt: new Date().toISOString() });
  await uploadAll(plan, dir, { env, dryRun, log });
  const { channel } = parseTag(tag);
  log(channel ? `${tag} is live at ${publicUrl(`${channel}/${INDEX}`)}` : `${tag} uploaded under ${publicUrl(`${tag}/`)} without moving a channel`);
  return plan;
}

const isMain = process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href;
if (isMain) {
  const args = process.argv.slice(2);
  const dryRun = args.includes("--dry-run");
  const filesMode = args.includes("--files");
  const [name, dir] = args.filter((arg) => arg !== "--dry-run" && arg !== "--files");
  if (!name || !dir) {
    console.error("usage: node deploy/releases/publish.mjs [--files] <tag-or-folder> <dir> [--dry-run]");
    process.exit(2);
  }
  const run = filesMode ? publishFiles({ folder: name, dir, dryRun }) : publish({ tag: name, dir, dryRun });
  run.catch((error) => {
    console.error(error.message);
    process.exit(1);
  });
}
