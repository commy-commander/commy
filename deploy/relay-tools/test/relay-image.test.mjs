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

const relayVersion = () => /^ARG IROH_RELAY_VERSION=(\S+)$/m.exec(read("relay/Dockerfile"))?.[1];
const productionSubnet = () =>
  /^\s+- subnet: \$\{RELAY_IPV6_SUBNET:-([0-9a-f:]+\/64)\}$/m.exec(read("docker-compose.relay.yml").replace(/\r/g, ""))?.[1];
const stagingRelayCommands = () =>
  [read("DEPLOYING.md"), read("setup-vps.sh")]
    .join("\n")
    .replace(/\r/g, "")
    .replace(/\\\n\s*/g, " ")
    .split("\n")
    .filter((line) => line.includes("-p commy-relay-staging"));

test("the shared relay network carries IPv6 on a private subnet", () => {
  const relay = read("docker-compose.relay.yml").replace(/\r/g, "");
  assert.match(relay, /^ {2}edge:\n {4}enable_ipv6: true\n/m);
  assert.match(productionSubnet() ?? "", /^fd[0-9a-f]{2}:/);
});

test("the staging relay has its own image tag and IPv6 subnet", () => {
  const commands = stagingRelayCommands();
  assert.ok(commands.length >= 4, "DEPLOYING.md and setup-vps.sh start the staging relay");
  for (const command of commands) {
    assert.ok(command.includes(`RELAY_IMAGE=commy-relay:staging-iroh-relay-${relayVersion()}`), command);
    const subnet = /RELAY_IPV6_SUBNET=(\S+)/.exec(command)?.[1];
    assert.match(subnet ?? "", /^fd[0-9a-f]{2}:.*\/64$/, command);
    assert.notEqual(subnet, productionSubnet(), command);
  }
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
