import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const workflow = readFileSync(new URL("../../../.github/workflows/mobile.yml", import.meta.url), "utf8").replace(/\r\n/g, "\n");

function stepRun(name) {
  const start = workflow.indexOf(`- name: ${name}`);
  assert.ok(start >= 0, `step "${name}" exists`);
  const next = workflow.indexOf("\n      - ", start + 1);
  return workflow.slice(start, next < 0 ? undefined : next);
}

test("a versioned simulator build takes the same servers and version as the signed build of that version", () => {
  const step = stepRun("Build the simulator app and an unsigned device app");
  assert.match(step, /if \[ -n "\$APP_VERSION_INPUT" \]; then/);
  assert.match(step, /source \.\.\/mobile\/release-env\.sh "\$APP_VERSION_INPUT"/);
  assert.match(step, /--config "\$\(printf '\{"version":"%s"\}' "\$release_app_version"\)"/);
  assert.match(step, /npx tauri ios build --ci --debug --no-sign --target aarch64-sim --target aarch64 "\$\{version_config\[@\]\}"/);
});

test("a versioned simulator build checks which servers it was built for", () => {
  const step = stepRun("Collect the apps");
  assert.match(step, /source apps\/desktop\/mobile\/release-env\.sh "\$APP_VERSION_INPUT"/);
  assert.match(step, /tar -cf - -C "\$app" \. \| expect_built_servers "The simulator app"/);
});
