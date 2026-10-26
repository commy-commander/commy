import assert from "node:assert/strict";
import net from "node:net";
import test from "node:test";

import { CommyLink, type TokenStore } from "../src/commy-link.ts";
import { LineBuffer } from "../src/lines.ts";

type Handler = (request: Record<string, unknown>, reply: (value: object) => void, socket: net.Socket) => void;

async function fakeCommy(handler: Handler): Promise<{ port: number; seen: Record<string, unknown>[]; close: () => void }> {
  const seen: Record<string, unknown>[] = [];
  const server = net.createServer(socket => {
    const lines = new LineBuffer();
    socket.setEncoding("utf8");
    socket.on("data", chunk => {
      for (const line of lines.push(String(chunk))) {
        const request = JSON.parse(line);
        seen.push(request);
        handler(request, value => socket.write(`${JSON.stringify(value)}\n`), socket);
      }
    });
    socket.on("error", () => {});
  });
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
  const port = (server.address() as net.AddressInfo).port;
  return { port, seen, close: () => server.close() };
}

function memoryTokens(initial: Record<number, string> = {}): TokenStore & { values: Record<number, string> } {
  const values = { ...initial };
  return {
    values,
    async load(port) {
      return values[port];
    },
    async save(port, token) {
      values[port] = token;
    },
    async forget(port) {
      delete values[port];
    },
  };
}

async function unusedPort(): Promise<number> {
  const server = net.createServer();
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
  const port = (server.address() as net.AddressInfo).port;
  await new Promise(resolve => server.close(resolve));
  return port;
}

function nextState(link: CommyLink, wanted: string): Promise<void> {
  return new Promise(resolve => {
    const stop = link.onState(state => {
      if (state === wanted) {
        stop();
        resolve();
      }
    });
  });
}

test("with nothing listening the link is offline", async () => {
  const link = new CommyLink({ tokens: memoryTokens(), ports: [await unusedPort()] });
  assert.equal(await link.connect(), "offline");
});

test("pairing waits for approval, then stores the token and becomes ready", async () => {
  const commy = await fakeCommy((request, reply) => {
    if (request.type === "hello") {
      reply({ type: "waiting" });
      setTimeout(() => {
        reply({ type: "paired", token: "fresh" });
        reply({ type: "ready", version: 1 });
      }, 20);
    }
  });
  const tokens = memoryTokens();
  const link = new CommyLink({ tokens, ports: [commy.port], name: "Desk" });
  const ready = nextState(link, "ready");
  assert.equal(await link.connect(), "waiting");
  await ready;
  assert.equal(tokens.values[commy.port], "fresh");
  assert.deepEqual(commy.seen[0], { type: "hello", version: 1, name: "Desk" });
  link.close();
  commy.close();
});

test("a stored token is sent and plays resolve or fail with Commy's reason", async () => {
  const commy = await fakeCommy((request, reply) => {
    if (request.type === "hello") reply({ type: "ready", version: 1 });
    if (request.type === "play" && request.sound === "boom") reply({ type: "played", id: request.id });
    if (request.type === "play" && request.sound === "gone") reply({ type: "failed", id: request.id, message: "that sound no longer exists" });
    if (request.type === "list") reply({ type: "soundboards", voice_server: null, defaults: [], servers: [] });
  });
  const tokens = memoryTokens({ [commy.port]: "known" });
  const known = new CommyLink({ tokens, ports: [commy.port] });
  assert.equal(await known.connect(), "ready");
  assert.equal(commy.seen[0].token, "known");
  await known.play("boom", "s1");
  await assert.rejects(known.play("gone"), /no longer exists/);
  assert.deepEqual(commy.seen.slice(1, 3), [
    { type: "play", id: 1, server: "s1", sound: "boom" },
    { type: "play", id: 2, sound: "gone" },
  ]);
  assert.deepEqual(await known.list(), { voice_server: null, defaults: [], servers: [] });
  known.close();
  commy.close();
});

test("adding a sound sends it with its server and returns what Commy stored", async () => {
  const commy = await fakeCommy((request, reply) => {
    if (request.type === "hello") reply({ type: "ready", version: 1 });
    if (request.type === "add_sound" && request.server === "s1") {
      reply({ type: "added", id: request.id, server: "s1", sound: { name: "boom", label: "boom", emoji: "💥" } });
    }
    if (request.type === "add_sound" && request.server === "s2") {
      reply({ type: "failed", id: request.id, message: "you do not have permission to add sounds" });
    }
  });
  const link = new CommyLink({ tokens: memoryTokens({ [commy.port]: "known" }), ports: [commy.port] });
  assert.equal(await link.connect(), "ready");
  const sound = await link.addSound({ server: "s1", name: "Boom", emoji: "💥", data: "AA==" });
  assert.deepEqual(sound, { name: "boom", label: "boom", emoji: "💥" });
  await assert.rejects(link.addSound({ server: "s2", name: "boom", emoji: null, data: "AA==" }), /permission/);
  assert.deepEqual(commy.seen.slice(1), [
    { type: "add_sound", id: 1, server: "s1", name: "Boom", data: "AA==", emoji: "💥" },
    { type: "add_sound", id: 2, server: "s2", name: "boom", data: "AA==" },
  ]);
  link.close();
  commy.close();
});

test("status, directory and controls come back from Commy", async () => {
  const status = { muted: true, unread: 2 };
  const commy = await fakeCommy((request, reply) => {
    if (request.type === "hello") reply({ type: "ready", version: 1 });
    if (request.type === "status") reply({ type: "status", ...status });
    if (request.type === "directory") reply({ type: "directory", servers: [], people: [], inputs: ["Desk mic"], outputs: [] });
    if (request.type === "control" && (request.control as { action: string }).action === "toggle_mute") {
      reply({ type: "done", id: request.id, status: { ...status, muted: false } });
    }
    if (request.type === "control" && (request.control as { action: string }).action === "toggle_camera") {
      reply({ type: "failed", id: request.id, message: "join a voice channel first" });
    }
  });
  const link = new CommyLink({ tokens: memoryTokens({ [commy.port]: "known" }), ports: [commy.port] });
  assert.equal(await link.connect(), "ready");
  assert.deepEqual(await link.status(), status);
  assert.deepEqual((await link.directory()).inputs, ["Desk mic"]);
  assert.equal((await link.control({ action: "toggle_mute" })).muted, false);
  await assert.rejects(link.control({ action: "toggle_camera" }), /voice channel/);
  assert.deepEqual(commy.seen.at(-1), { type: "control", id: 2, control: { action: "toggle_camera" } });
  link.close();
  commy.close();
});

test("a declined deck reports it and a removed one forgets its token", async () => {
  const declining = await fakeCommy((request, reply, socket) => {
    reply({ type: "denied", reason: "declined" });
    socket.end();
  });
  assert.equal(await new CommyLink({ tokens: memoryTokens(), ports: [declining.port] }).connect(), "declined");
  declining.close();

  const removing = await fakeCommy((request, reply, socket) => {
    if (request.type === "hello") reply({ type: "ready", version: 1 });
    if (request.type === "list") {
      reply({ type: "denied", reason: "removed" });
      socket.end();
    }
  });
  const tokens = memoryTokens({ [removing.port]: "old" });
  const link = new CommyLink({ tokens, ports: [removing.port] });
  assert.equal(await link.connect(), "ready");
  await assert.rejects(link.list());
  assert.equal(link.state, "declined");
  assert.equal(tokens.values[removing.port], undefined);
  removing.close();
});

test("the second port is tried when the first has no Commy", async () => {
  const commy = await fakeCommy((request, reply) => reply({ type: "ready", version: 1 }));
  const link = new CommyLink({ tokens: memoryTokens(), ports: [await unusedPort(), commy.port] });
  assert.equal(await link.connect(), "ready");
  link.close();
  commy.close();
});

test("closing the connection goes offline and fails waiting requests", async () => {
  const commy = await fakeCommy((request, reply, socket) => {
    if (request.type === "hello") reply({ type: "ready", version: 1 });
    if (request.type === "play") socket.destroy();
  });
  const link = new CommyLink({ tokens: memoryTokens(), ports: [commy.port] });
  await link.connect();
  await assert.rejects(link.play("boom"), /closed/);
  assert.equal(link.state, "offline");
  commy.close();
});

test("requests before connecting are refused", async () => {
  const link = new CommyLink({ tokens: memoryTokens(), ports: [] });
  await assert.rejects(link.play("boom"), /not connected/);
});
