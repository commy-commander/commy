// Sits between Caddy and iroh-relay. Plain requests (captive portal checks)
// pass straight through; relay upgrades get their 101 response fixed so
// Caddy will switch protocols, and are refused while the meter has paused
// the relay for the month.
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import { fixUpgradeHead, requestHead } from "./lib.mjs";

const UPSTREAM_HOST = process.env.RELAY_HOST || "relay";
const UPSTREAM_PORT = Number(process.env.RELAY_PORT || 3340);
const LISTEN_PORT = Number(process.env.PROXY_PORT || 3341);
const FLAG_FILE = process.env.FLAG_FILE || "/flags/relay-paused";
const MAX_HEAD = 16 * 1024;

const paused = () => fs.existsSync(FLAG_FILE);

const server = http.createServer((req, res) => {
  const upstream = http.request(
    { host: UPSTREAM_HOST, port: UPSTREAM_PORT, method: req.method, path: req.url, headers: req.headers },
    (up) => {
      res.writeHead(up.statusCode ?? 502, up.headers);
      up.pipe(res);
    },
  );
  upstream.on("error", () => {
    if (!res.headersSent) res.writeHead(502);
    res.end();
  });
  req.pipe(upstream);
});

server.on("upgrade", (req, client, head) => {
  if (paused()) {
    const body = "Relay paused for the rest of the month\n";
    client.end(`HTTP/1.1 503 Service Unavailable\r\nContent-Length: ${body.length}\r\nConnection: close\r\n\r\n${body}`);
    return;
  }
  const upstream = net.connect(UPSTREAM_PORT, UPSTREAM_HOST);
  const fail = () => {
    upstream.destroy();
    client.destroy();
  };
  upstream.on("error", fail);
  client.on("error", fail);

  upstream.once("connect", () => {
    upstream.write(requestHead(req.method, req.url, req.rawHeaders));
    if (head.length) upstream.write(head);
  });

  let buffered = Buffer.alloc(0);
  const onData = (chunk) => {
    buffered = Buffer.concat([buffered, chunk]);
    const end = buffered.indexOf("\r\n\r\n");
    if (end < 0) {
      if (buffered.length > MAX_HEAD) fail();
      return;
    }
    upstream.off("data", onData);
    client.write(fixUpgradeHead(buffered.subarray(0, end + 4).toString("latin1")), "latin1");
    const rest = buffered.subarray(end + 4);
    if (rest.length) client.write(rest);
    upstream.pipe(client);
    client.pipe(upstream);
  };
  upstream.on("data", onData);
});

server.listen(LISTEN_PORT, () => console.log(`[relay-proxy] :${LISTEN_PORT} -> ${UPSTREAM_HOST}:${UPSTREAM_PORT}`));
