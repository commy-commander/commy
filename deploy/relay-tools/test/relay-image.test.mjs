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
