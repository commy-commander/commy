import { EventEmitter } from "node:events";
import { readFile } from "node:fs/promises";
import { createConnection } from "node:net";
import { join } from "node:path";

export const DEFAULT_PORT = 47620;
const TOKEN_FILE = "bridge.token";

function embedJson(embed) {
  return embed && typeof embed.toJSON === "function" ? embed.toJSON() : embed;
}

export function outgoing(input) {
  if (typeof input === "string") return { content: input, embeds: [] };
  const { content = null, embeds = [], embed } = input || {};
  const all = embed ? [...embeds, embed] : embeds;
  return { content, embeds: all.map(embedJson) };
}

export function parseCommand(content, prefix) {
  if (!prefix || !content.startsWith(prefix)) return null;
  const words = content.slice(prefix.length).trim().split(/\s+/).filter(Boolean);
  if (!words.length) return null;
  return { name: words[0].toLowerCase(), args: words.slice(1) };
}

class Roles {
  constructor(member) {
    this.member = member;
  }

  get ids() {
    return [...this.member.roleIds];
  }

  has(nameOrId) {
    const role = this.member.server.role(nameOrId);
    return Boolean(role && this.member.roleIds.has(role.id));
  }

  async add(nameOrId) {
    const role = this.member.server.role(nameOrId);
    await this.member.client.request("add_role", { server: this.member.server.id, user: this.member.id, role: role ? role.id : String(nameOrId) });
    if (role) this.member.roleIds.add(role.id);
  }

  async remove(nameOrId) {
    const role = this.member.server.role(nameOrId);
    await this.member.client.request("remove_role", { server: this.member.server.id, user: this.member.id, role: role ? role.id : String(nameOrId) });
    if (role) this.member.roleIds.delete(role.id);
  }
}

export class Member {
  constructor(client, server, data) {
    this.client = client;
    this.server = server;
    this.id = data.id;
    this.name = data.name;
    this.username = data.username || data.name;
    this.bot = Boolean(data.bot);
    this.owner = Boolean(data.owner);
    this.roleIds = new Set(data.roles || []);
    this.roles = new Roles(this);
  }

  toString() {
    return `<@${this.id}>`;
  }

  kick() {
    return this.client.request("kick", { server: this.server.id, user: this.id });
  }

  ban() {
    return this.client.request("ban", { server: this.server.id, user: this.id });
  }
}

export class Channel {
  constructor(client, server, data) {
    this.client = client;
    this.server = server;
    this.id = data.id;
    this.name = data.name;
    this.kind = data.kind || "text";
  }

  toString() {
    return `<#${this.id}>`;
  }

  async send(input) {
    const result = await this.client.request("send", { server: this.server.id, channel: this.id, ...outgoing(input) });
    return new Message(this.client, this.server, result.message);
  }
}

export class Server {
  constructor(client, data) {
    this.client = client;
    this.id = data.id;
    this.name = data.name;
    this.ready = data.ready !== false;
    this.ownerId = data.owner || null;
    this.channels = new Map((data.channels || []).map(c => [c.id, new Channel(client, this, c)]));
    this.roles = new Map((data.roles || []).map(r => [r.id, { ...r, toString: () => `<@&${r.id}>` }]));
    this.members = new Map((data.members || []).map(m => [m.id, new Member(client, this, m)]));
  }

  channel(nameOrId) {
    const key = String(nameOrId || "").replace(/^#/, "").toLowerCase();
    return this.channels.get(nameOrId) || [...this.channels.values()].find(c => c.name.toLowerCase() === key) || null;
  }

  role(nameOrId) {
    const key = String(nameOrId || "").replace(/^@/, "").toLowerCase();
    return this.roles.get(nameOrId) || [...this.roles.values()].find(r => r.name.toLowerCase() === key) || null;
  }

  member(id) {
    return this.members.get(id) || null;
  }
}

export class Message {
  constructor(client, server, data) {
    this.client = client;
    this.server = server;
    this.id = data.id;
    this.content = data.content || "";
    this.channel = server.channel(data.channel) || new Channel(client, server, { id: data.channel, name: data.channel_name || "" });
    this.author = server.member(data.author.id) || new Member(client, server, data.author);
    this.member = this.author;
    this.createdAt = new Date(data.created_at);
    this.editedAt = data.edited_at ? new Date(data.edited_at) : null;
    this.replyTo = data.reply_to || null;
    this.attachments = data.attachments || [];
    this.reactions = data.reactions || [];
    this.mentions = (data.mentions || []).map(id => server.member(id)).filter(Boolean);
    this.late = Boolean(data.late);
  }

  async reply(input) {
    const result = await this.client.request("send", { server: this.server.id, channel: this.channel.id, reply_to: this.id, ...outgoing(input) });
    return new Message(this.client, this.server, result.message);
  }

  edit(input) {
    return this.client.request("edit", { server: this.server.id, message: this.id, ...outgoing(input) });
  }

  delete() {
    return this.client.request("delete", { server: this.server.id, message: this.id });
  }

  react(emoji) {
    return this.client.request("react", { server: this.server.id, message: this.id, emoji });
  }

  unreact(emoji) {
    return this.client.request("unreact", { server: this.server.id, message: this.id, emoji });
  }
}

export class Client extends EventEmitter {
  constructor({ prefix = "!", includeLate = false } = {}) {
    super();
    this.prefix = prefix;
    this.includeLate = includeLate;
    this.user = null;
    this.servers = new Map();
    this.commands = new Map();
    this.socket = null;
    this.pending = new Map();
    this.nextNonce = 1;
  }

  command(name, run) {
    this.commands.set(String(name).toLowerCase(), run);
    return this;
  }

  server(nameOrId) {
    const key = String(nameOrId || "").toLowerCase();
    return this.servers.get(nameOrId) || [...this.servers.values()].find(s => s.name.toLowerCase() === key) || null;
  }

  async login({ token, data, port = DEFAULT_PORT, host = "127.0.0.1" } = {}) {
    const secret = token || process.env.COMMY_BOT_TOKEN || (await readToken(data || process.env.COMMY_BOT_DATA || "commy-bot-data"));
    const socket = createConnection({ host, port });
    socket.setEncoding("utf8");
    socket.setNoDelay(true);
    this.socket = socket;
    return new Promise((resolve, reject) => {
      let buffer = "";
      let signedIn = false;
      socket.on("connect", () => socket.write(`${JSON.stringify({ op: "hello", token: secret })}\n`));
      socket.on("data", chunk => {
        buffer += chunk;
        let at;
        while ((at = buffer.indexOf("\n")) >= 0) {
          const line = buffer.slice(0, at);
          buffer = buffer.slice(at + 1);
          if (!line.trim()) continue;
          const event = JSON.parse(line);
          if (!signedIn) {
            if (event.type !== "hello") {
              socket.destroy();
              return reject(new Error(event.error || "the bridge refused the connection"));
            }
            signedIn = true;
            this.welcome(event);
            resolve(this);
            continue;
          }
          this.receive(event);
        }
      });
      socket.on("error", error => (signedIn ? this.emit("error", error) : reject(error)));
      socket.on("close", () => {
        for (const { reject: fail } of this.pending.values()) fail(new Error("the bridge closed"));
        this.pending.clear();
        if (!signedIn) reject(new Error("the bridge closed before signing in"));
        else this.emit("close");
      });
    });
  }

  destroy() {
    if (this.socket) this.socket.end();
  }

  request(op, fields) {
    if (!this.socket) return Promise.reject(new Error("not signed in; call login first"));
    const nonce = this.nextNonce++;
    return new Promise((resolve, reject) => {
      this.pending.set(nonce, { resolve, reject });
      this.socket.write(`${JSON.stringify({ op, nonce, ...fields })}\n`);
    });
  }

  async refresh(serverId) {
    const result = await this.request("server", { server: serverId });
    const server = new Server(this, result.server);
    this.servers.set(server.id, server);
    return server;
  }

  welcome(hello) {
    this.user = hello.me;
    for (const data of hello.servers) this.servers.set(data.id, new Server(this, data));
    this.emit("ready", this);
  }

  receive(event) {
    if (event.type === "result") return this.settle(event);
    if (event.type === "server_ready") {
      const server = new Server(this, event.server);
      this.servers.set(server.id, server);
      return this.emit("serverReady", server);
    }
    if (event.type === "lagged") return this.emit("warn", `missed ${event.missed} events`);
    const server = this.servers.get(event.server);
    if (!server) return;
    switch (event.type) {
      case "message_create":
      case "message_update":
        return this.onMessage(event.type, new Message(this, server, event.message));
      case "message_delete":
        return this.emit("messageDelete", { id: event.message, channel: server.channel(event.channel), server });
      case "reaction_add":
      case "reaction_remove": {
        const user = server.member(event.user.id) || new Member(this, server, event.user);
        const name = event.type === "reaction_add" ? "messageReactionAdd" : "messageReactionRemove";
        return this.emit(name, { emoji: event.emoji, messageId: event.message, channel: server.channel(event.channel), server }, user);
      }
      case "member_join": {
        const member = new Member(this, server, event.member);
        server.members.set(member.id, member);
        return this.emit("memberJoin", member);
      }
      case "member_leave": {
        const member = server.member(event.user) || { id: event.user, server };
        server.members.delete(event.user);
        return this.emit("memberLeave", member);
      }
    }
  }

  onMessage(type, message) {
    if (message.late && !this.includeLate) return;
    if (type === "message_update") return this.emit("messageUpdate", message);
    this.emit("messageCreate", message);
    const command = parseCommand(message.content, this.prefix);
    const run = command && this.commands.get(command.name);
    if (!run) return;
    Promise.resolve()
      .then(() => run(message, command.args))
      .catch(error => this.emit("error", error));
  }

  settle(result) {
    const waiting = this.pending.get(result.nonce);
    if (!waiting) return;
    this.pending.delete(result.nonce);
    if (result.ok) waiting.resolve(result);
    else waiting.reject(new Error(result.error));
  }
}

async function readToken(folder) {
  try {
    return (await readFile(join(folder, TOKEN_FILE), "utf8")).trim();
  } catch {
    throw new Error(`no bridge token in ${folder}; run commy-bot run --bridge once, or pass { token }`);
  }
}
