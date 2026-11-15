import assert from "node:assert/strict";
import fs from "node:fs";
import { test } from "node:test";

const deploy = new URL("../../", import.meta.url);
const read = (path) => fs.readFileSync(new URL(path, deploy), "utf8");

test("the relay image tag names the iroh-relay version it builds", () => {
  const version = /^ARG IROH_RELAY_VERSION=(\S+)$/m.exec(read("relay/Dockerfile"))?.[1];
  const image = /^\s+image: \$\{RELAY_IMAGE:-([^}]+)\}$/m.exec(read("docker-compose.yml"))?.[1];
  assert.ok(version, "Dockerfile pins IROH_RELAY_VERSION");
  assert.equal(image, `commy-relay:iroh-relay-${version}`);
});

test("the relay publishes the address discovery port it listens on", () => {
  const port = /^\s+- "\$\{RELAY_QAD_PORT:-7842\}:(\d+)\/udp"$/m.exec(read("docker-compose.yml"))?.[1];
  const bind = /quic_bind_addr = \\"\[::\]:(\d+)\\"/.exec(read("relay/run-relay.sh"))?.[1];
  assert.equal(port, "7842");
  assert.equal(bind, port);
});

test("the shared relay reads the edge certificate read only", () => {
  assert.match(read("docker-compose.relay.yml"), /- edge_caddy_data:\/caddy-data:ro/);
  assert.match(read("docker-compose.relay.yml"), /name: commy-edge_caddy_data/);
});

test("each relay client sends at most a few megabytes a second", () => {
  const config = read("relay/iroh-relay.toml").replace(/\r/g, "");
  const table = config.slice(config.indexOf("[limits.client.rx]"));
  const rate = Number(/^bytes_per_second = (\d+)$/m.exec(table)?.[1]);
  const burst = Number(/^max_burst_bytes = (\d+)$/m.exec(table)?.[1]);
  assert.ok(config.includes("[limits.client.rx]"));
  assert.ok(rate > 0 && rate <= 8_000_000, "one client cannot drain the monthly allowance at line speed");
  assert.ok(burst >= rate);
});
