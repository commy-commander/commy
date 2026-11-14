import { test } from "node:test";
import assert from "node:assert/strict";
import {
  COPIED_FILES,
  compareTags,
  copyPlan,
  copyReleases,
  gitPushCommand,
  isStableTag,
  removeRepeatedHeadings,
  stableReleases,
  tagsToPush,
} from "./github-copy.mjs";

const release = (tag, extra = {}) => ({ tag_name: tag, name: `Commy ${tag}`, body: `notes ${tag}`, draft: false, prerelease: false, ...extra });
const copy = (tag, extra = {}) => ({ id: Number(tag.replace(/\D/g, "")), body: `notes ${tag}`, assets: [...COPIED_FILES], ...extra });

test("only final release tags count as stable", () => {
  assert.equal(isStableTag("v0.2.4"), true);
  assert.equal(isStableTag("v0.2.4-rc.1"), false);
  assert.equal(isStableTag("v0.2.4-test.1"), false);
  assert.equal(isStableTag("tts-pack-1"), false);
});

test("drafts, pre-releases and non-version tags are never copied", () => {
  const kept = stableReleases([
    release("v0.2.3"),
    release("v0.2.4", { draft: true }),
    release("v0.2.5-rc.1", { prerelease: true }),
    release("tts-pack-1"),
  ]);
  assert.deepEqual(kept.map((r) => r.tag_name), ["v0.2.3"]);
});

test("tags sort by version, not by text", () => {
  const tags = ["v0.2.10", "v0.2.9", "v0.10.0", "v0.2.1"];
  assert.deepEqual([...tags].sort(compareTags), ["v0.2.1", "v0.2.9", "v0.2.10", "v0.10.0"]);
});

test("a heading repeated later in the notes is dropped with its blank line", () => {
  const body = "## Downloads\n\nEvery file is here.\n\n## Downloads\n\n| Platform | File |\n";
  assert.equal(removeRepeatedHeadings(body), "## Downloads\n\nEvery file is here.\n\n| Platform | File |\n");
});

test("notes without repeated headings are unchanged", () => {
  const body = "## What's new in 0.2.4\n\n**New**\n- Item\n\n## Downloads\n\nFiles.";
  assert.equal(removeRepeatedHeadings(body), body);
  assert.equal(removeRepeatedHeadings(undefined), "");
});

test("missing releases are created oldest first and only the newest is marked latest", () => {
  const steps = copyPlan({ forge: [release("v0.2.4"), release("v0.2.2"), release("v0.2.3")], github: new Map([["v0.2.2", copy("v0.2.2")]]) });
  assert.deepEqual(steps.map((s) => [s.tag, s.action, s.latest]), [
    ["v0.2.3", "create", false],
    ["v0.2.4", "create", true],
  ]);
  assert.deepEqual(steps[1].files, COPIED_FILES);
  assert.deepEqual(tagsToPush(steps), ["v0.2.3", "v0.2.4"]);
});

test("changed notes are updated in place without pushing anything", () => {
  const steps = copyPlan({ forge: [release("v0.2.4", { body: "## What's new\n\nMore" })], github: new Map([["v0.2.4", copy("v0.2.4")]]) });
  assert.deepEqual(steps, [{ tag: "v0.2.4", action: "update", id: 24, body: "## What's new\n\nMore" }]);
  assert.deepEqual(tagsToPush(steps), []);
});

test("notes that differ only by a repeated heading count as unchanged", () => {
  const forgeBody = "## Downloads\n\nA\n\n## Downloads\n\nB";
  const steps = copyPlan({ forge: [release("v0.2.4", { body: forgeBody })], github: new Map([["v0.2.4", copy("v0.2.4", { body: "## Downloads\n\nA\n\nB" })]]) });
  assert.deepEqual(steps, []);
});

test("a copy missing some files gets only those files", () => {
  const steps = copyPlan({ forge: [release("v0.2.4")], github: new Map([["v0.2.4", copy("v0.2.4", { assets: ["latest.json"] })]]) });
  assert.deepEqual(steps, [{ tag: "v0.2.4", action: "upload", id: 24, files: ["SHA256SUMS.txt", "SHA256SUMS.txt.bundle"] }]);
});

test("nothing to do when GitHub matches the forge", () => {
  assert.deepEqual(copyPlan({ forge: [release("v0.2.4")], github: new Map([["v0.2.4", copy("v0.2.4")]]) }), []);
  assert.deepEqual(copyPlan({ forge: [], github: new Map() }), []);
});

test("the push sends main and the new tags with the token outside the arguments", () => {
  const push = gitPushCommand({ repo: "owner/app", token: "secret-value", tags: ["v0.2.4"] });
  assert.deepEqual(push.args, ["push", "https://github.com/owner/app.git", "HEAD:refs/heads/main", "refs/tags/v0.2.4"]);
  assert.ok(!push.args.join(" ").includes("secret-value"));
  const header = push.env.GIT_CONFIG_VALUE_0;
  assert.equal(push.env.GIT_CONFIG_KEY_0, "http.https://github.com/.extraheader");
  assert.equal(Buffer.from(header.replace("AUTHORIZATION: basic ", ""), "base64").toString(), "x-access-token:secret-value");
  assert.equal(push.args.includes("--force"), false);
});

function fakeServer({ forge, github = {}, hosted = {} }) {
  const calls = [];
  let nextId = 100;
  const json = (value, status = 200) => new Response(JSON.stringify(value), { status });
  const fetch = async (url, options = {}) => {
    const method = options.method ?? "GET";
    calls.push({ method, url, headers: options.headers ?? {}, body: options.body });
    const path = new URL(url);
    if (path.host === "git.commy.chat") return json(forge);
    if (path.host === "releases.commy.chat") {
      const key = path.pathname.slice(1);
      return key in hosted ? new Response(hosted[key]) : new Response("missing", { status: 404 });
    }
    if (path.host === "uploads.github.com") return json({}, 201);
    const tagMatch = path.pathname.match(/\/releases\/tags\/(.+)$/);
    if (method === "GET" && tagMatch) {
      const found = github[decodeURIComponent(tagMatch[1])];
      return found ? json(found) : json({ message: "Not Found" }, 404);
    }
    if (method === "POST" && path.pathname.endsWith("/releases")) {
      const created = JSON.parse(options.body);
      return json({ id: nextId++, html_url: `https://github.com/x/releases/tag/${created.tag_name}` }, 201);
    }
    if (method === "PATCH") return json({});
    throw new Error(`unexpected ${method} ${url}`);
  };
  return { fetch, calls };
}

test("a new forge release is pushed, created and given its three files", async () => {
  const server = fakeServer({
    forge: [release("v0.2.4"), release("v0.2.3")],
    github: { "v0.2.3": { id: 23, body: "notes v0.2.3", assets: COPIED_FILES.map((name) => ({ name })) } },
    hosted: { "v0.2.4/latest.json": "{}", "v0.2.4/SHA256SUMS.txt": "sums" },
  });
  const pushes = [];
  const logs = [];
  await copyReleases({
    env: { MIRROR_GITHUB_TOKEN: "t0ken", GITHUB_SERVER_URL: "https://git.commy.chat", GITHUB_REPOSITORY: "commy/commy" },
    fetch: server.fetch,
    git: (args, env) => pushes.push({ args, env }),
    log: (line) => logs.push(line),
  });
  assert.equal(pushes.length, 1);
  assert.deepEqual(pushes[0].args.slice(2), ["HEAD:refs/heads/main", "refs/tags/v0.2.4"]);
  const created = server.calls.find((c) => c.method === "POST" && c.url.endsWith("/releases"));
  assert.deepEqual(JSON.parse(created.body), {
    tag_name: "v0.2.4",
    name: "Commy v0.2.4",
    body: "notes v0.2.4",
    draft: false,
    prerelease: false,
    make_latest: "true",
  });
  const uploads = server.calls.filter((c) => c.url.startsWith("https://uploads.github.com"));
  assert.deepEqual(uploads.map((c) => new URL(c.url).searchParams.get("name")), ["latest.json", "SHA256SUMS.txt"]);
  assert.equal(uploads[0].headers["Content-Type"], "application/json");
  assert.ok(logs.some((line) => line.includes("SHA256SUMS.txt.bundle is not on releases.commy.chat")));
  assert.ok(server.calls.filter((c) => c.url.includes("api.github.com")).every((c) => c.headers.Authorization === "Bearer t0ken"));
});

test("edited notes on the forge only patch the GitHub release", async () => {
  const server = fakeServer({
    forge: [release("v0.2.4", { body: "## What's new in 0.2.4" })],
    github: { "v0.2.4": { id: 24, body: "notes v0.2.4", assets: COPIED_FILES.map((name) => ({ name })) } },
  });
  const pushes = [];
  await copyReleases({ env: { MIRROR_GITHUB_TOKEN: "t" }, fetch: server.fetch, git: (args) => pushes.push(args), log: () => {} });
  assert.equal(pushes.length, 0);
  const patch = server.calls.find((c) => c.method === "PATCH");
  assert.match(patch.url, /\/releases\/24$/);
  assert.deepEqual(JSON.parse(patch.body), { body: "## What's new in 0.2.4" });
});

test("without a token nothing is read or pushed", async () => {
  const server = fakeServer({ forge: [release("v0.2.4")] });
  const logs = [];
  const steps = await copyReleases({ env: {}, fetch: server.fetch, git: () => assert.fail("pushed"), log: (line) => logs.push(line) });
  assert.deepEqual(steps, []);
  assert.equal(server.calls.length, 0);
  assert.match(logs[0], /MIRROR_GITHUB_TOKEN is not set/);
});

test("a dry run reports the plan and changes nothing", async () => {
  const server = fakeServer({ forge: [release("v0.2.4")] });
  const steps = await copyReleases({ env: {}, dryRun: true, fetch: server.fetch, git: () => assert.fail("pushed"), log: () => {} });
  assert.deepEqual(steps.map((s) => s.action), ["create"]);
  assert.ok(server.calls.every((c) => c.method === "GET"));
  assert.ok(server.calls.every((c) => !("Authorization" in c.headers) || c.url.includes("git.commy.chat")));
});

test("a failed GitHub call stops the copy with its status", async () => {
  const fetch = async (url) => (url.includes("git.commy.chat") ? new Response(JSON.stringify([release("v0.2.4")])) : new Response("boom", { status: 500 }));
  await assert.rejects(copyReleases({ env: { MIRROR_GITHUB_TOKEN: "t" }, fetch, git: () => {}, log: () => {} }), /reading GitHub release v0.2.4 failed: 500 boom/);
});
