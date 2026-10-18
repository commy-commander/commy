import net from "node:net";

import { LineBuffer } from "./lines.ts";
import {
  COMMY_HOST,
  COMMY_PORTS,
  encodeRequest,
  parseReply,
  PROTOCOL_VERSION,
  type Reply,
  type Request,
  type SoundboardList,
} from "./protocol.ts";

export type LinkState = "offline" | "waiting" | "declined" | "busy" | "unsupported" | "ready";

export type TokenStore = {
  load(port: number): Promise<string | undefined>;
  save(port: number, token: string): Promise<void>;
  forget(port: number): Promise<void>;
};

export type LinkOptions = {
  tokens: TokenStore;
  name?: string;
  ports?: number[];
  host?: string;
  connectTimeoutMs?: number;
  replyTimeoutMs?: number;
};

type Waiter = { resolve: (reply: Reply) => void; reject: (error: Error) => void; timer: NodeJS.Timeout };

export class CommyLink {
  #tokens: TokenStore;
  #name: string;
  #ports: number[];
  #host: string;
  #connectTimeoutMs: number;
  #replyTimeoutMs: number;
  #state: LinkState = "offline";
  #socket: net.Socket | null = null;
  #connecting: Promise<LinkState> | null = null;
  #listeners = new Set<(state: LinkState) => void>();
  #plays = new Map<number, Waiter>();
  #lists: Waiter[] = [];
  #nextId = 1;

  constructor(options: LinkOptions) {
    this.#tokens = options.tokens;
    this.#name = options.name ?? "Stream Deck";
    this.#ports = options.ports ?? COMMY_PORTS;
    this.#host = options.host ?? COMMY_HOST;
    this.#connectTimeoutMs = options.connectTimeoutMs ?? 1500;
    this.#replyTimeoutMs = options.replyTimeoutMs ?? 5000;
  }

  get state(): LinkState {
    return this.#state;
  }

  onState(listener: (state: LinkState) => void): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  connect(): Promise<LinkState> {
    if (this.#state === "ready" || this.#state === "waiting") return Promise.resolve(this.#state);
    this.#connecting ??= this.#connectAny().finally(() => {
      this.#connecting = null;
    });
    return this.#connecting;
  }

  async list(): Promise<SoundboardList> {
    const reply = await this.#request({ type: "list" }, waiter => this.#lists.push(waiter));
    if (reply.type !== "soundboards") throw new Error("unexpected reply");
    const { type: _type, ...list } = reply;
    return list;
  }

  async play(sound: string, server?: string): Promise<void> {
    const id = this.#nextId++;
    const request: Request = server ? { type: "play", id, server, sound } : { type: "play", id, sound };
    const reply = await this.#request(request, waiter => this.#plays.set(id, waiter));
    if (reply.type === "failed") throw new Error(reply.message);
  }

  close(): void {
    this.#socket?.destroy();
  }

  async #connectAny(): Promise<LinkState> {
    for (const port of this.#ports) {
      const state = await this.#connectTo(port);
      if (state !== "offline") return state;
    }
    this.#setState("offline");
    return "offline";
  }

  async #connectTo(port: number): Promise<LinkState> {
    const socket = await this.#open(port);
    if (!socket) return "offline";
    const token = await this.#tokens.load(port);
    return new Promise<LinkState>(resolve => {
      let settled = false;
      const settle = (state: LinkState) => {
        this.#setState(state);
        if (!settled) {
          settled = true;
          resolve(state);
        }
      };
      const lines = new LineBuffer();
      this.#socket = socket;
      socket.setEncoding("utf8");
      socket.on("data", chunk => {
        let received: string[];
        try {
          received = lines.push(String(chunk));
        } catch {
          socket.destroy();
          return;
        }
        for (const line of received) {
          const reply = parseReply(line);
          if (reply) this.#handle(port, reply, settle);
        }
      });
      socket.on("close", () => {
        if (this.#socket === socket) this.#socket = null;
        this.#failAll(new Error("Commy closed the connection"));
        if (!settled || this.#state === "ready" || this.#state === "waiting") settle("offline");
      });
      socket.on("error", () => socket.destroy());
      const hello: Request = { type: "hello", version: PROTOCOL_VERSION, name: this.#name };
      if (token) hello.token = token;
      socket.write(encodeRequest(hello));
    });
  }

  #handle(port: number, reply: Reply, settle: (state: LinkState) => void): void {
    switch (reply.type) {
      case "waiting":
        settle("waiting");
        break;
      case "paired":
        void this.#tokens.save(port, reply.token);
        break;
      case "ready":
        settle("ready");
        break;
      case "denied":
        if (reply.reason === "removed") void this.#tokens.forget(port);
        settle(reply.reason === "removed" ? "declined" : reply.reason);
        break;
      case "soundboards":
        this.#lists.shift()?.resolve(reply);
        break;
      case "played":
      case "failed":
        this.#plays.get(reply.id)?.resolve(reply);
        this.#plays.delete(reply.id);
        break;
    }
  }

  #open(port: number): Promise<net.Socket | null> {
    return new Promise(resolve => {
      const socket = net.connect({ host: this.#host, port });
      const timer = setTimeout(() => {
        socket.destroy();
        resolve(null);
      }, this.#connectTimeoutMs);
      socket.once("connect", () => {
        clearTimeout(timer);
        socket.removeAllListeners("error");
        socket.setNoDelay(true);
        resolve(socket);
      });
      socket.once("error", () => {
        clearTimeout(timer);
        socket.destroy();
        resolve(null);
      });
    });
  }

  #request(request: Request, enqueue: (waiter: Waiter) => void): Promise<Reply> {
    const socket = this.#socket;
    if (this.#state !== "ready" || !socket) return Promise.reject(new Error("Commy is not connected"));
    return new Promise<Reply>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#forget(waiter);
        reject(new Error("Commy did not answer"));
      }, this.#replyTimeoutMs);
      const waiter: Waiter = {
        resolve: reply => {
          clearTimeout(timer);
          resolve(reply);
        },
        reject: error => {
          clearTimeout(timer);
          reject(error);
        },
        timer,
      };
      enqueue(waiter);
      socket.write(encodeRequest(request));
    });
  }

  #forget(waiter: Waiter): void {
    this.#lists = this.#lists.filter(w => w !== waiter);
    for (const [id, w] of this.#plays) if (w === waiter) this.#plays.delete(id);
  }

  #failAll(error: Error): void {
    for (const waiter of [...this.#lists, ...this.#plays.values()]) waiter.reject(error);
    this.#lists = [];
    this.#plays.clear();
  }

  #setState(state: LinkState): void {
    if (state === this.#state) return;
    this.#state = state;
    for (const listener of this.#listeners) listener(state);
  }
}
