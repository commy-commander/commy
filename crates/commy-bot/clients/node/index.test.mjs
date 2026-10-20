import assert from "node:assert/strict";
import { once } from "node:events";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { Client, outgoing, parseCommand } from "./index.mjs";

const SERVER = {
  id: "s1",
  name: "Hangout",
  ready: true,
  owner: "u-owner",
  channels: [{ id: "c1", name: "general", kind: "text" }],
  roles: [{ id: "r1", name: "Helpers", position: 1 }],
  members: [
    { id: "u-owner", name: "owner", bot: false, roles: [] },
    { id: "u-bot", name: "Tidy", bot: true, roles: [] },
    { id: "u-ana", name: "ana", bot: false, roles: [] },
  ],
};

function message(fields = {}) {
  return {
    id: "m1",
    server: "s1",
    channel: "c1",
    channel_name: "general",
    author: { id: "u-ana", name: "ana", bot: false, roles: [] },
    content: "hello",
    created_at: "2026-10-20T12:00:00Z",
    edited_at: null,
    reply_to: null,
    attachments: [],
    reactions: [],
    mentions: [],
    late: false,
    ...fields,
  };
}

async function fakeBridge({ token = "secret", answer = () => ({ ok: true }) } = {}) {
  const requests = [];
  let peer = null;
  const server = createServer(socket => {
    peer = socket;
    socket.setEncoding("utf8");
    let buffer = "";
    let signedIn = false;
    socket.on("data", chunk => {
      buffer += chunk;
      let at;
      while ((at = buffer.indexOf("\n")) >= 0) {
        const request = JSON.parse(buffer.slice(0, at));
        buffer = buffer.slice(at + 1);
        if (!signedIn) {
          if (request.op !== "hello" || request.token !== token) {
            socket.end(`${JSON.stringify({ type: "denied", error: "wrong token" })}\n`);
            return;
          }
          signedIn = true;
          socket.write(`${JSON.stringify({ type: "hello", version: 1, me: { id: "u-bot", name: "Tidy" }, servers: [SERVER] })}\n`);
          continue;
        }
        requests.push(request);
        socket.write(`${JSON.stringify({ type: "result", nonce: request.nonce, ...answer(request) })}\n`);
      }
    });
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  return {
    port: server.address().port,
    requests,
    push: event => peer.write(`${JSON.stringify(event)}\n`),
    close: () => {
      if (peer) peer.destroy();
      server.close();
    },
  };
}

test("commands split on the prefix", () => {
  assert.deepEqual(parseCommand("!Roll 20 extra", "!"), { name: "roll", args: ["20", "extra"] });
  assert.equal(parseCommand("hello", "!"), null);
  assert.equal(parseCommand("!", "!"), null);
});

test("outgoing accepts text, embeds and builder objects", () => {
  assert.deepEqual(outgoing("hi"), { content: "hi", embeds: [] });
  const built = { toJSON: () => ({ title: "Built" }) };
  assert.deepEqual(outgoing({ embeds: [built], embed: { title: "Plain" } }), { content: null, embeds: [{ title: "Built" }, { title: "Plain" }] });
});

test("login reads the token from the data folder and refuses a wrong one", async () => {
  const bridge = await fakeBridge();
  const folder = await mkdtemp(join(tmpdir(), "commy-bot-"));
  try {
    await writeFile(join(folder, "bridge.token"), "secret\n");
    const client = new Client();
    const ready = once(client, "ready");
    await client.login({ data: folder, port: bridge.port });
    await ready;
    assert.equal(client.user.name, "Tidy");
    assert.equal(client.server("hangout").channel("#general").id, "c1");
    client.destroy();
    await assert.rejects(new Client().login({ token: "guess", port: bridge.port }), /wrong token/);
  } finally {
    bridge.close();
    await rm(folder, { recursive: true, force: true });
  }
});

test("messages run commands, reply through the bridge and skip late ones", async () => {
  const bridge = await fakeBridge({
    answer: request => (request.op === "send" ? { ok: true, message: message({ id: "m2", author: { id: "u-bot", name: "Tidy", bot: true }, content: request.content }) } : { ok: true }),
  });
  try {
    const client = new Client();
    const seen = [];
    client.on("messageCreate", m => seen.push(m.content));
    let replied;
    const done = new Promise(resolve => {
      client.command("ping", async m => {
        replied = await m.reply({ content: `pong ${m.author}`, embeds: [{ title: "ok" }] });
        resolve();
      });
    });
    await client.login({ token: "secret", port: bridge.port });
    bridge.push({ type: "message_create", server: "s1", message: message({ id: "old", content: "!ping", late: true }) });
    bridge.push({ type: "message_create", server: "s1", message: message({ content: "!ping", mentions: ["u-bot"] }) });
    await done;
    assert.deepEqual(seen, ["!ping"]);
    const send = bridge.requests.find(r => r.op === "send");
    assert.deepEqual(
      { server: send.server, channel: send.channel, reply_to: send.reply_to, content: send.content, embeds: send.embeds },
      { server: "s1", channel: "c1", reply_to: "m1", content: "pong <@u-ana>", embeds: [{ title: "ok" }] },
    );
    assert.equal(replied.id, "m2");
    client.destroy();
  } finally {
    bridge.close();
  }
});

test("failed actions reject with the bridge's reason and members stay in step", async () => {
  const bridge = await fakeBridge({
    answer: request => (request.op === "ban" ? { ok: false, error: "the bot needs a role with Ban Members" } : { ok: true }),
  });
  try {
    const client = new Client();
    await client.login({ token: "secret", port: bridge.port });
    const server = client.server("s1");
    const joined = once(client, "memberJoin");
    bridge.push({ type: "member_join", server: "s1", member: { id: "u-new", name: "newbie", bot: false, roles: [] } });
    const [member] = await joined;
    assert.equal(server.member("u-new"), member);
    await assert.rejects(member.ban(), /Ban Members/);
    await member.roles.add("helpers");
    assert.ok(member.roles.has("r1"));
    assert.deepEqual(bridge.requests.at(-1), { op: "add_role", nonce: 2, server: "s1", user: "u-new", role: "r1" });
    const left = once(client, "memberLeave");
    bridge.push({ type: "member_leave", server: "s1", user: "u-new" });
    await left;
    assert.equal(server.member("u-new"), null);
    const reacted = once(client, "messageReactionAdd");
    bridge.push({ type: "reaction_add", server: "s1", channel: "c1", message: "m1", user: { id: "u-ana", name: "ana" }, emoji: "👍" });
    const [reaction, user] = await reacted;
    assert.equal(reaction.emoji, "👍");
    assert.equal(user.name, "ana");
    client.destroy();
  } finally {
    bridge.close();
  }
});
