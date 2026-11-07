import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  IMMUTABLE,
  accountIdProblem,
  POINTER,
  contentType,
  encodeKey,
  filesPlan,
  manifestProblems,
  parseTag,
  publish,
  publishFiles,
  releaseIndex,
  signRequest,
  uploadPlan,
} from "./publish.mjs";

const files = [
  { name: "Commy_0.2.1_x64-setup.exe", size: 100 },
  { name: "Commy_0.2.1_universal.app.tar.gz", size: 200 },
  { name: "SHA256SUMS.txt", size: 3 },
  { name: "commy-soundboard.streamDeckPlugin", size: 4 },
  { name: "latest.json", size: 5 },
];

const manifest = (tag, version) => ({
  version,
  platforms: {
    "windows-x86_64": { url: `https://releases.commy.chat/${tag}/Commy_${version}_x64-setup.exe`, signature: "sig" },
    "darwin-aarch64": { url: `https://releases.commy.chat/${tag}/Commy_${version}_universal.app.tar.gz`, signature: "sig" },
  },
});

test("finals go to stable, release candidates to rc, test tags to no channel", () => {
  assert.deepEqual(parseTag("v0.2.1"), { tag: "v0.2.1", version: "0.2.1", prerelease: false, channel: "stable" });
  assert.deepEqual(parseTag("v0.2.1-rc.3"), { tag: "v0.2.1-rc.3", version: "0.2.1-rc.3", prerelease: true, channel: "rc" });
  assert.equal(parseTag("v0.2.1-test.1").channel, null);
  assert.equal(parseTag("v0.2.1-rc.3.test").channel, null);
});

test("anything that is not a v-prefixed version is refused", () => {
  for (const tag of ["0.2.1", "latest", "v0.2", "", undefined, "v0.2.1/../x"]) assert.throws(() => parseTag(tag));
});

test("versioned files are immutable and the channel pointers move last", () => {
  const plan = uploadPlan({ tag: "v0.2.1", files, publishedAt: "2026-11-06T20:00:00Z" });
  const keys = plan.map((step) => step.key);
  assert.deepEqual(keys.slice(0, files.length), files.map((file) => `v0.2.1/${file.name}`));
  assert.deepEqual(keys.slice(files.length), ["stable/commy-soundboard.streamDeckPlugin", "stable/release.json", "stable/latest.json"]);
  assert.ok(plan.slice(0, files.length).every((step) => step.cacheControl === IMMUTABLE));
  assert.ok(plan.slice(files.length).every((step) => step.cacheControl === POINTER));
});

test("a test tag uploads its files without touching a channel", () => {
  const plan = uploadPlan({ tag: "v0.2.1-test.1", files, publishedAt: "2026-11-06T20:00:00Z" });
  assert.ok(plan.every((step) => step.key.startsWith("v0.2.1-test.1/")));
});

test("without a signed manifest the channel keeps its old update pointer", () => {
  const unsigned = files.filter((file) => file.name !== "latest.json");
  const keys = uploadPlan({ tag: "v0.2.1-rc.1", files: unsigned, publishedAt: "x" }).map((step) => step.key);
  assert.ok(keys.includes("rc/release.json"));
  assert.ok(!keys.includes("rc/latest.json"));
});

test("the release index has the shape the download page reads", () => {
  const index = releaseIndex({ tag: "v0.2.1", publishedAt: "2026-11-06T20:00:00Z", files });
  assert.equal(index.tag_name, "v0.2.1");
  assert.equal(index.prerelease, false);
  assert.equal(index.html_url, "https://commy.chat/download");
  assert.deepEqual(index.assets[0], {
    name: "Commy_0.2.1_x64-setup.exe",
    browser_download_url: "https://releases.commy.chat/v0.2.1/Commy_0.2.1_x64-setup.exe",
    size: 100,
  });
});

test("a manifest that matches the release has no problems", () => {
  assert.deepEqual(manifestProblems(manifest("v0.2.1", "0.2.1"), "v0.2.1", files.map((f) => f.name)), []);
});

test("a manifest for another version or host is caught", () => {
  const names = files.map((f) => f.name);
  assert.equal(manifestProblems(manifest("v0.2.1", "0.2.0"), "v0.2.1", names).length > 0, true);
  const elsewhere = manifest("v0.2.1", "0.2.1");
  elsewhere.platforms["windows-x86_64"].url = "https://example.test/v0.2.1/Commy_0.2.1_x64-setup.exe";
  assert.match(manifestProblems(elsewhere, "v0.2.1", names).join(), /outside/);
  const missing = manifest("v0.2.1", "0.2.1");
  missing.platforms["darwin-aarch64"].url = "https://releases.commy.chat/v0.2.1/Commy_0.2.1_missing.tar.gz";
  assert.match(manifestProblems(missing, "v0.2.1", names).join(), /not in this release/);
  const unsigned = manifest("v0.2.1", "0.2.1");
  unsigned.platforms["windows-x86_64"].signature = "";
  assert.match(manifestProblems(unsigned, "v0.2.1", names).join(), /no signature/);
  assert.match(manifestProblems({ version: "0.2.1" }, "v0.2.1", names).join(), /no platforms/);
});

test("installers get their download types", () => {
  assert.equal(contentType("Commy_0.2.1_x64-setup.exe"), "application/vnd.microsoft.portable-executable");
  assert.equal(contentType("Commy_0.2.1_universal.app.tar.gz"), "application/gzip");
  assert.equal(contentType("latest.json"), "application/json");
  assert.equal(contentType("commy-soundboard.streamDeckPlugin"), "application/octet-stream");
});

test("keys keep their slashes and escape everything S3 wants escaped", () => {
  assert.equal(encodeKey("v0.2.1/Commy (1).exe"), "v0.2.1/Commy%20%281%29.exe");
});

test("signatures match the published S3 signature version 4 example", () => {
  const headers = signRequest({
    method: "GET",
    url: "https://examplebucket.s3.amazonaws.com/test.txt",
    headers: { range: "bytes=0-9" },
    accessKeyId: "AKIAIOSFODNN7EXAMPLE",
    secretAccessKey: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
    region: "us-east-1",
    now: new Date("2013-05-24T00:00:00Z"),
  });
  assert.equal(
    headers.authorization,
    "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41",
  );
});

test("a dry run reads the dist folder and refuses a bad manifest", async () => {
  const dir = await mkdtemp(join(tmpdir(), "commy-release-"));
  await writeFile(join(dir, "Commy_0.2.1_x64-setup.exe"), "exe");
  await writeFile(join(dir, "Commy_0.2.1_universal.app.tar.gz"), "app");
  await writeFile(join(dir, "latest.json"), JSON.stringify(manifest("v0.2.1", "0.2.1")));
  const plan = await publish({ tag: "v0.2.1", dir, dryRun: true, log: () => {} });
  assert.equal(plan.at(-1).key, "stable/latest.json");
  await writeFile(join(dir, "latest.json"), JSON.stringify(manifest("v0.2.1", "0.2.0")));
  await assert.rejects(publish({ tag: "v0.2.1", dir, dryRun: true, log: () => {} }), /refusing to publish/);
});

test("other files go under their own immutable folder", () => {
  const plan = filesPlan("tts-pack-1", [{ name: "piper_phonemize.wasm", size: 1 }]);
  assert.deepEqual(plan, [{ key: "tts-pack-1/piper_phonemize.wasm", file: "piper_phonemize.wasm", cacheControl: IMMUTABLE, contentType: "application/octet-stream" }]);
});

test("a file folder can never overwrite a channel or climb out of the bucket", () => {
  for (const folder of ["stable", "rc", "../x", "a/b", "", undefined, ".hidden"]) assert.throws(() => filesPlan(folder, []));
});

test("a file folder dry run lists every file", async () => {
  const dir = await mkdtemp(join(tmpdir(), "commy-files-"));
  await writeFile(join(dir, "voice.onnx"), "v");
  const plan = await publishFiles({ folder: "tts-pack-1", dir, dryRun: true, log: () => {} });
  assert.deepEqual(plan.map((step) => step.key), ["tts-pack-1/voice.onnx"]);
});

test("the account ID must be the bare 32 character ID", () => {
  assert.equal(accountIdProblem("0123456789abcdef0123456789abcdef"), null);
  assert.match(accountIdProblem(" 0123456789abcdef0123456789abcdef\n"), /spaces or line breaks/);
  assert.match(accountIdProblem("0123456789abcdef"), /is 16 hex characters/);
  assert.match(accountIdProblem("https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com"), /not all hex/);
});

test("a malformed account ID is refused before any upload", async () => {
  const dir = await mkdtemp(join(tmpdir(), "commy-release-"));
  await writeFile(join(dir, "SHA256SUMS.txt"), "x");
  const env = { R2_ACCOUNT_ID: "nope", R2_ACCESS_KEY_ID: "a", R2_SECRET_ACCESS_KEY: "b" };
  await assert.rejects(publish({ tag: "v0.2.1", dir, env, log: () => {} }), /R2_ACCOUNT_ID should be/);
});

test("a real run needs the R2 credentials", async () => {
  const dir = await mkdtemp(join(tmpdir(), "commy-release-"));
  await writeFile(join(dir, "SHA256SUMS.txt"), "x");
  await assert.rejects(publish({ tag: "v0.2.1", dir, env: {}, log: () => {} }), /missing R2_ACCOUNT_ID/);
});
