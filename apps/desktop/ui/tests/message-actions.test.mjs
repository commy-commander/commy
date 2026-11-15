import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";

const require = createRequire(import.meta.url);
const { QUICK_MESSAGE_ACTION_COUNT, splitMessageActions, messageMenuLabel } = require("../scripts/message-actions.js");
const { menuSpot, menuFitHeight } = require("../scripts/menu-spot.js");
const read = (...parts) => readFileSync(new URL(`../${parts.join("/")}`, import.meta.url), "utf8");
const app = read("scripts", "app.js");
const html = read("index.html");

const actions = ids => ids.map(id => ({ id, title: id }));
const ids = list => list.map(action => action.id);

test("the hover bar keeps four actions and sends the rest to the More menu", () => {
  assert.equal(QUICK_MESSAGE_ACTION_COUNT, 4);
  const own = actions(["react", "reply", "thread", "edit", "pin", "save", "remind", "report", "delete"]);
  const { quick, rest } = splitMessageActions(own);
  assert.deepEqual(ids(quick), ["react", "reply", "edit", "thread"]);
  assert.deepEqual(ids(rest), ["pin", "save", "remind", "report", "delete"]);
});

test("someone else's message fills the bar from the next most used actions", () => {
  const theirs = actions(["react", "reply", "pin", "save", "remind", "hide", "report", "delete"]);
  const { quick, rest } = splitMessageActions(theirs);
  assert.deepEqual(ids(quick), ["react", "reply", "save", "pin"]);
  assert.deepEqual(ids(rest), ["remind", "hide", "report", "delete"]);
});

test("delete, report, hide and remind never take a hover bar slot", () => {
  const { quick, rest } = splitMessageActions(actions(["hide", "remind", "report", "delete"]));
  assert.deepEqual(quick, []);
  assert.deepEqual(ids(rest), ["hide", "remind", "report", "delete"]);
  assert.deepEqual(splitMessageActions(null), { quick: [], rest: [] });
});

test("menu rows use the longer label when an action has one", () => {
  assert.equal(messageMenuLabel({ title: "Delete", label: "Delete Message" }), "Delete Message");
  assert.equal(messageMenuLabel({ title: "Remind Me" }), "Remind Me");
});

test("the hover bar is built from the split and the More menu gets the rest", () => {
  assert.match(app, /const \{ quick, rest \} = splitMessageActions\(actions\);/);
  assert.match(app, /add\("delete", "trash", own \? "Delete" : "Delete \(Moderator\)"/);
  assert.match(app, /tail: true, danger: true \}\);/);
  assert.match(html, /<script src="scripts\/menu-spot\.js"><\/script>\n\s*<script src="scripts\/message-actions\.js"><\/script>/);
});

const view = { viewWidth: 940, viewHeight: 700 };

test("a menu with room below opens under its anchor", () => {
  assert.deepEqual(menuSpot({ ...view, x: 300, y: 200, flipY: 180, width: 220, height: 300 }), { left: 300, top: 200 });
});

test("a menu with no room below flips above its anchor instead of running off the window", () => {
  assert.deepEqual(menuSpot({ ...view, x: 300, y: 600, flipY: 580, width: 220, height: 300 }), { left: 300, top: 280 });
});

test("a menu with no room either side sits against the bottom edge", () => {
  assert.deepEqual(menuSpot({ ...view, x: 300, y: 600, flipY: 200, width: 220, height: 300 }), { left: 300, top: 392 });
  assert.deepEqual(menuSpot({ ...view, x: 300, y: 600, width: 220, height: 300 }), { left: 300, top: 392 });
});

test("a menu never starts past the right edge or above the top edge", () => {
  assert.deepEqual(menuSpot({ ...view, x: 900, y: 650, flipY: 640, width: 220, height: 900 }), { left: 712, top: 8 });
});

test("a menu taller than the window gets a height that scrolls", () => {
  assert.equal(menuFitHeight(300, 700), null);
  assert.equal(menuFitHeight(684, 700), null);
  assert.equal(menuFitHeight(900, 700), 684);
  assert.match(app, /menu\.style\.overflowY = "auto";/);
  assert.match(app, /width: menu\.offsetWidth, height: menu\.offsetHeight/);
});
