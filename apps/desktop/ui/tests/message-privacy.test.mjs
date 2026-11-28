import "./i18n-global.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const { DAY_KEY_CHOICES, dayKeysDaysLabel, dayKeysStatus, dayKeysWarnsOlderApps, dayKeysControls, canSeeMessagePrivacy } = require("../scripts/message-privacy.js");

const view = fields => ({ on: false, chosen: false, since: null, history_days: 30, history_choices: [1, 7, 30, 90], waiting: 0, can_turn_on: true, can_turn_off: true, ...fields });
const when = at => `day ${at}`;

test("the status says whether daily keys are on, off by choice, or waiting on members to update", () => {
  assert.equal(dayKeysStatus(view({ on: true, chosen: true, since: 5 }), when), "On since day 5.");
  assert.match(dayKeysStatus(view({ chosen: true }), when), /owner turned daily keys off/);
  assert.match(dayKeysStatus(view({ waiting: 0 }), when), /turn on in a moment/);
  assert.match(dayKeysStatus(view({ waiting: 1 }), when), /Waiting on 1 member to update/);
  assert.match(dayKeysStatus(view({ waiting: 4 }), when), /Waiting on 4 members to update/);
  assert.equal(dayKeysStatus(null, when), "");
});

test("the older apps note shows only while someone could still switch it on early", () => {
  assert.ok(dayKeysWarnsOlderApps(view({ waiting: 2 })));
  assert.ok(!dayKeysWarnsOlderApps(view({ waiting: 0 })));
  assert.ok(!dayKeysWarnsOlderApps(view({ on: true, waiting: 2 })));
  assert.ok(!dayKeysWarnsOlderApps(view({ waiting: 2, can_turn_on: false })));
});

test("managers turn it on, only the owner turns it off, and the read back choice keeps to the four lengths", () => {
  const manager = view({ can_turn_off: false });
  assert.equal(dayKeysControls(manager).toggleDisabled, false);
  assert.equal(dayKeysControls({ ...manager, on: true }).toggleDisabled, true);
  assert.equal(dayKeysControls(view({ on: true })).toggleDisabled, false);
  assert.equal(dayKeysControls(view({ can_turn_on: false })).historyDisabled, true);
  assert.deepEqual(dayKeysControls(view({ history_choices: [] })).choices, DAY_KEY_CHOICES);
  assert.equal(dayKeysControls(view({ history_days: 3 })).days, 30);
  assert.equal(dayKeysControls(view({ history_days: 7 })).days, 7);
  assert.equal(dayKeysDaysLabel(1), "1 day");
  assert.equal(dayKeysDaysLabel(90), "90 days");
});

test("the page shows for people who manage a server, not for group chats or plain members", () => {
  assert.ok(canSeeMessagePrivacy({ day_keys: view({}) }));
  assert.ok(!canSeeMessagePrivacy({ group: true, day_keys: view({}) }));
  assert.ok(!canSeeMessagePrivacy({ day_keys: view({ can_turn_on: false }) }));
  assert.ok(!canSeeMessagePrivacy({ day_keys: null }));
});

test("the read back lengths match the ones the core accepts", () => {
  const source = readFileSync(join(import.meta.dirname, "..", "..", "..", "..", "crates", "commy-core", "src", "day_chain.rs"), "utf8");
  const listed = /HISTORY_DAYS: \[u32; 4\] = \[([^\]]+)\]/.exec(source)[1].split(",").map(n => Number(n.trim()));
  assert.deepEqual(listed, DAY_KEY_CHOICES);
});
