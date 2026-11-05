import test from "node:test";
import assert from "node:assert/strict";
import { handle, SEALED_SIZES, MAX_FILE_PLAIN, objectName, fitsSealed } from "../worker.js";

function memoryBucket() {
  const objects = new Map();
  return {
    objects,
    async head(name) {
      const body = objects.get(name);
      return body ? { size: body.byteLength } : null;
    },
    async get(name) {
      const body = objects.get(name);
      return body ? { size: body.byteLength, body: new Uint8Array(body) } : null;
    },
    async put(name, body) {
      const bytes = new Uint8Array(await new Response(body).arrayBuffer());
      objects.set(name, bytes);
      return { size: bytes.byteLength };
    },
    async delete(name) {
      objects.delete(name);
    },
  };
}

const id = "ab".repeat(32);
const at = path => `https://commy-files.rockets.workers.dev${path}`;

function send(env, method, path, body, headers = {}) {
  const sized = body ? { "content-length": String(body.byteLength), ...headers } : headers;
  return handle(new Request(at(path), { method, body, headers: sized }), env);
}

test("the root says this is Commy file storage and how big a file may be", async () => {
  const res = await send({ FILES: memoryBucket() }, "GET", "/");
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), { commy_files: 1, max_bytes: MAX_FILE_PLAIN });
  assert.equal(res.headers.get("access-control-allow-origin"), "*");
});

test("a sealed file is kept once and read back by its id", async () => {
  const env = { FILES: memoryBucket() };
  assert.equal((await send(env, "HEAD", `/${id}`)).status, 404);
  assert.equal((await send(env, "GET", `/${id}`)).status, 404);
  const sealed = new Uint8Array(SEALED_SIZES[0]).fill(7);
  assert.equal((await send(env, "PUT", `/${id}`, sealed)).status, 204);
  assert.equal((await send(env, "HEAD", `/${id}`)).status, 200);
  const res = await send(env, "GET", `/${id}`);
  assert.equal(res.status, 200);
  assert.equal(res.headers.get("cache-control"), "public, max-age=31536000, immutable");
  assert.deepEqual(new Uint8Array(await res.arrayBuffer()), sealed);
  const other = new Uint8Array(SEALED_SIZES[0]).fill(9);
  assert.equal((await send(env, "PUT", `/${id}`, other)).status, 204);
  assert.deepEqual(env.FILES.objects.get(objectName(id)), sealed, "a kept file is never replaced");
});

test("only whole sealed sizes and real ids are taken", async () => {
  const env = { FILES: memoryBucket() };
  assert.equal((await send(env, "PUT", `/${id}`, new Uint8Array(1000))).status, 400);
  assert.equal((await send(env, "PUT", "/not-an-id", new Uint8Array(SEALED_SIZES[0]))).status, 404);
  assert.equal((await send(env, "GET", `/${"AB".repeat(32)}`)).status, 404);
  assert.equal((await send(env, "DELETE", `/${id}`)).status, 405);
  assert.equal((await send(env, "POST", "/")).status, 405);
  const sealed = new Uint8Array(SEALED_SIZES[1]);
  assert.equal((await send(env, "PUT", `/${id}`, sealed, { "content-length": String(SEALED_SIZES[0]) })).status, 400, "a body that is not the size it claims is dropped");
  assert.equal(env.FILES.objects.size, 0);
});

test("browsers may ask first", async () => {
  const res = await send({ FILES: memoryBucket() }, "OPTIONS", `/${id}`);
  assert.equal(res.status, 204);
  assert.match(res.headers.get("access-control-allow-methods"), /PUT/);
});

test("a worker without its bucket says so", async () => {
  assert.equal((await send({}, "GET", "/")).status, 503);
});

test("sealed sizes match the app's buckets", () => {
  assert.equal(SEALED_SIZES.length, 10);
  assert.ok(fitsSealed(64 * 1024 + 16));
  assert.ok(fitsSealed(30 * 1024 * 1024 + 4 + 16));
  assert.ok(fitsSealed(24 * 1024 * 1024 + 16));
  assert.ok(!fitsSealed(64 * 1024));
});
