import { createSign } from "node:crypto";
import { readFile } from "node:fs/promises";
import { basename } from "node:path";
import { pathToFileURL } from "node:url";

export const PACKAGE = "chat.commy.mobile";
export const TRACKS = { internal: "internal", closed: "alpha", open: "beta" };
const SCOPE = "https://www.googleapis.com/auth/androidpublisher";
const API = "https://androidpublisher.googleapis.com/androidpublisher/v3/applications";
const UPLOAD_API = "https://androidpublisher.googleapis.com/upload/androidpublisher/v3/applications";

const base64url = data => Buffer.from(data).toString("base64url");

export function readServiceAccount(text) {
  let key;
  try {
    key = JSON.parse(text);
  } catch {
    throw new Error("PLAY_SERVICE_ACCOUNT_JSON is not valid JSON");
  }
  for (const field of ["client_email", "private_key"]) {
    if (typeof key[field] !== "string" || !key[field]) throw new Error(`PLAY_SERVICE_ACCOUNT_JSON has no ${field}`);
  }
  return { email: key.client_email, privateKey: key.private_key, tokenUri: key.token_uri || "https://oauth2.googleapis.com/token" };
}

export function signedAssertion(account, now = Math.floor(Date.now() / 1000)) {
  const header = base64url(JSON.stringify({ alg: "RS256", typ: "JWT" }));
  const claims = base64url(JSON.stringify({ iss: account.email, scope: SCOPE, aud: account.tokenUri, iat: now, exp: now + 3600 }));
  const signer = createSign("RSA-SHA256");
  signer.update(`${header}.${claims}`);
  return `${header}.${claims}.${signer.sign(account.privateKey).toString("base64url")}`;
}

export function versionCodeOf(file) {
  const match = basename(file).match(/-(\d+)\.aab$/);
  if (!match) throw new Error(`can't read the version code from ${basename(file)}`);
  return match[1];
}

export function trackRelease({ track, name, versionCode, status, notes }) {
  const release = { name, versionCodes: [String(versionCode)], status };
  if (notes) release.releaseNotes = [{ language: "en-US", text: notes.slice(0, 500) }];
  return { track, releases: [release] };
}

async function call(fetchImpl, url, init, what) {
  const response = await fetchImpl(url, init);
  const text = await response.text();
  if (!response.ok) {
    let message = text;
    try {
      message = JSON.parse(text).error?.message || text;
    } catch {}
    const error = new Error(`${what} failed (${response.status}): ${message}`);
    error.status = response.status;
    error.detail = message;
    throw error;
  }
  return text ? JSON.parse(text) : {};
}

export async function uploadBundle({ account, bundle, versionCode, track: testing = "internal", name, notes, packageName = PACKAGE, fetch: fetchImpl = fetch, log = console.log }) {
  const track = Object.hasOwn(TRACKS, testing) ? TRACKS[testing] : null;
  if (!track) throw new Error(`the testing track must be one of ${Object.keys(TRACKS).join(", ")}`);
  const token = await call(fetchImpl, account.tokenUri, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({ grant_type: "urn:ietf:params:oauth:grant-type:jwt-bearer", assertion: signedAssertion(account) }).toString(),
  }, "Signing in to Google");
  const auth = { authorization: `Bearer ${token.access_token}` };
  const app = `${API}/${packageName}`;
  const edit = await call(fetchImpl, `${app}/edits`, { method: "POST", headers: { ...auth, "content-type": "application/json" }, body: "{}" }, "Opening a Play edit");
  const uploaded = await call(fetchImpl, `${UPLOAD_API}/${packageName}/edits/${edit.id}/bundles?uploadType=media`, {
    method: "POST",
    headers: { ...auth, "content-type": "application/octet-stream" },
    body: bundle,
  }, "Uploading the bundle");
  const code = String(uploaded.versionCode ?? versionCode);
  log(`Uploaded version code ${code}`);
  const putTrack = status => call(fetchImpl, `${app}/edits/${edit.id}/tracks/${track}`, {
    method: "PUT",
    headers: { ...auth, "content-type": "application/json" },
    body: JSON.stringify(trackRelease({ track, name, versionCode: code, status, notes })),
  }, `Adding it to ${testing} testing`);
  let status = "completed";
  try {
    await putTrack(status);
  } catch (error) {
    if (!/draft app/i.test(error.detail || "")) throw error;
    status = "draft";
    log("Play still lists Commy as a draft app, so this release is saved as a draft. Roll it out in Play Console.");
    await putTrack(status);
  }
  await call(fetchImpl, `${app}/edits/${edit.id}:commit`, { method: "POST", headers: auth }, "Publishing the edit");
  return { versionCode: code, track: testing, status };
}

async function main() {
  const [file, track = "internal"] = process.argv.slice(2);
  if (!file) throw new Error("usage: play-upload.mjs BUNDLE.aab [internal|closed|open]");
  const account = readServiceAccount(process.env.PLAY_SERVICE_ACCOUNT_JSON || "");
  const name = process.env.PLAY_RELEASE_NAME || basename(file, ".aab");
  const result = await uploadBundle({
    account,
    bundle: await readFile(file),
    versionCode: versionCodeOf(file),
    track,
    name,
    notes: process.env.PLAY_RELEASE_NOTES || "",
  });
  console.log(`Version code ${result.versionCode} is in ${result.track} testing (${result.status})`);
}

if (import.meta.url === pathToFileURL(process.argv[1] || "").href) {
  main().catch(error => {
    console.log(`::error::${error.message}`);
    process.exit(1);
  });
}
