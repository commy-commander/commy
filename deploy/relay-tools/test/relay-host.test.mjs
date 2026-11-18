import assert from "node:assert/strict";
import fs from "node:fs";
import { test } from "node:test";

const deploy = new URL("../../", import.meta.url);
const read = (path) => fs.readFileSync(new URL(path, deploy), "utf8").replace(/\r/g, "");

const block = (text, opening) => {
  const start = text.indexOf(`${opening} {\n`);
  assert.notEqual(start, -1, `missing block ${opening}`);
  const end = text.indexOf("\n}\n", start);
  return text.slice(start, end + 3);
};

const relayBlocks = (caddyfile) =>
  ["(relay)", "{$RELAY_DOMAIN}:8443", "{$STAGING_RELAY_DOMAIN}:8443", "https://:8443"].map((opening) =>
    block(caddyfile, opening),
  );

test("the relay server serves the relays exactly like the main edge", () => {
  assert.deepEqual(relayBlocks(read("relay-host/Caddyfile")), relayBlocks(read("edge/Caddyfile")));
});

test("the relay server edge serves nothing but the relays", () => {
  const caddyfile = read("relay-host/Caddyfile");
  const sites = caddyfile.match(/^\S.* \{$/gm);
  assert.ok(caddyfile.startsWith("{\n\temail {$ACME_EMAIL}\n}\n"));
  assert.deepEqual(sites, ["(relay) {", "{$RELAY_DOMAIN}:8443 {", "{$STAGING_RELAY_DOMAIN}:8443 {", "https://:8443 {"]);
});

test("the relay server edge publishes only the challenge and relay ports", () => {
  const compose = read("relay-host/compose.yml");
  assert.deepEqual(compose.match(/^\s+- "[^"]+:\d+(\/\w+)?"$/gm)?.map((line) => line.trim()), [
    '- "80:80"',
    '- "8443:8443"',
  ]);
});

test("the relay server edge provides the network and certificates the relays expect", () => {
  const compose = read("relay-host/compose.yml");
  const relay = read("docker-compose.relay.yml");
  const publicNetwork = /^ {2}public:\n {4}external: true\n {4}name: (\S+)$/m.exec(relay)?.[1];
  const certificates = /^ {2}edge_caddy_data:\n {4}external: true\n {4}name: (\S+)$/m.exec(relay)?.[1];
  assert.equal(/^ {2}relay:\n {4}name: (\S+)$/m.exec(compose)?.[1], publicNetwork);
  assert.equal(/^ {2}caddy_data:\n {4}name: (\S+)$/m.exec(compose)?.[1], certificates);
  assert.match(compose, /^ {4}networks: \[relay\]$/m);
  assert.match(compose, /^ {6}- caddy_data:\/data$/m);
});

test("the relay server edge is locked down like the main edge", () => {
  const compose = read("relay-host/compose.yml");
  assert.match(compose, /security_opt: \["no-new-privileges:true"\]/);
  assert.match(compose, /cap_drop: \[ALL\]/);
  assert.match(compose, /cap_add: \[NET_BIND_SERVICE\]/);
  assert.match(compose, /- \.\/Caddyfile:\/etc\/caddy\/Caddyfile:ro/);
});

test("the access checker keeps its edge name on every shared network", () => {
  const relay = read("docker-compose.relay.yml");
  const access = relay.slice(relay.indexOf("  relay-access:\n"), relay.indexOf("\nnetworks:\n"));
  assert.match(access, /public: &relay_access_alias\n {8}aliases: \["\$\{RELAY_ACCESS_ALIAS:-commy-relay-access\}"\]/);
  assert.match(access, /accounts: \*relay_access_alias/);
  assert.match(access, /test-accounts: \*relay_access_alias/);
});

test("the test build access checker never takes the release relay's name", () => {
  const commands = read("DEPLOYING.md")
    .replace(/\\\n\s*/g, " ")
    .split("\n")
    .filter((line) => line.includes("-p commy-relay-staging") && / relay-access$/.test(line));
  assert.ok(commands.length >= 1);
  for (const command of commands) {
    assert.ok(command.includes("RELAY_ACCESS_ALIAS=commy-relay-staging-access"), command);
  }
});

test("a moved release relay asks the release access checker", () => {
  const moved = read("DEPLOYING.md")
    .replace(/\\\n\s*/g, " ")
    .split("\n")
    .filter((line) => line.includes("-p commy-relay ") && line.includes("--no-deps"));
  assert.equal(moved.length, 1);
  assert.match(moved[0], /RELAY_ACCESS_URL=https:\/\/\S+:8443\/prod\//);
  assert.match(moved[0], /RELAY_QAD_PORT=7842/);
  assert.match(moved[0], /RELAY_IPV6_SUBNET=fd99:6513:f442:1::\/64/);
});

test("the edge routes release relay checks apart from test build ones", () => {
  const site = read("edge/sites.d/relay-access.caddy");
  assert.match(site, /^relay-access\.\{\$PROD_DOMAIN\}:8443 \{$/m);
  assert.match(site, /remote_ip \{\$RELAY_HOST_IPS:127\.0\.0\.1\/32\}/);
  assert.match(site, /handle \/prod\/\* \{\n\t\t\treverse_proxy commy-relay-access:3342/);
  assert.match(site, /reverse_proxy commy-relay-staging-access:3342/);
  assert.doesNotMatch(site, /\d+\.\d+\.\d+\.\d+(?!\/32\})/);
  assert.match(read("edge/docker-compose.yml"), /RELAY_HOST_IPS: \$\{RELAY_HOST_IPS:-127\.0\.0\.1\/32\}/);
  assert.match(read("edge/.gitignore"), /^!sites\.d\/relay-access\.caddy$/m);
});

test("a moved relay asks the access checker over HTTPS without starting one", () => {
  const moved = read("DEPLOYING.md")
    .replace(/\\\n\s*/g, " ")
    .split("\n")
    .filter((line) => line.includes("-p commy-relay-staging") && line.includes("--no-deps"));
  assert.equal(moved.length, 1);
  assert.match(moved[0], /RELAY_ACCESS_URL=https:\/\/\S+:8443\//);
  assert.match(moved[0], /up -d --no-deps relay relay-proxy relay-meter$/);
});
