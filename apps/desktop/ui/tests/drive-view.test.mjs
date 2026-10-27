import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const {
  DRIVE_NAME_MAX,
  DRIVE_DEPTH_MAX,
  isDriveChannel,
  driveFolderOf,
  driveChildren,
  driveCrumbs,
  driveInside,
  driveMoveTargets,
  driveCleanName,
  driveSize,
  driveUnavailable,
  driveKeptLabel,
  driveRoomLeft,
} = require("../scripts/drive-view.js");

const root = join(import.meta.dirname, "..", "..", "..", "..");
const read = path => readFileSync(join(root, path), "utf8");

const folder = (id, parent, name) => ({ id, parent, name, folder: true });
const file = (id, parent, name, extra = {}) => ({ id, parent, name, folder: false, size: 10, keepers: 1, keepers_online: 1, kept_here: false, ...extra });

const items = [
  folder("maps", null, "Maps"),
  folder("old", "maps", "old"),
  file("a", "maps", "b-route.png"),
  file("b", "maps", "A-route.png"),
  file("c", null, "rules.pdf"),
  folder("clips", null, "clips"),
];

test("the app and core agree on drive limits and the channel kind", () => {
  const core = read("crates/commy-core/src/drive.rs");
  assert.match(core, new RegExp(`MAX_DRIVE_NAME: usize = ${DRIVE_NAME_MAX};`));
  assert.match(core, new RegExp(`MAX_DRIVE_DEPTH: usize = ${DRIVE_DEPTH_MAX};`));
  assert.match(read("crates/commy-view/src/manage.rs"), /"drive" => Ok\(ChannelKind::Drive\)/);
  assert.match(read("apps/desktop/ui/index.html"), /<script src="scripts\/drive-view\.js"><\/script>/);
});

test("every drive command the app calls is registered", () => {
  const app = read("apps/desktop/ui/scripts/app.js");
  const lib = read("apps/desktop/src-tauri/src/lib.rs");
  const called = new Set([...app.matchAll(/invoke\("(drive_\w+)"/g)].map(m => m[1]));
  assert.ok(called.size >= 7);
  for (const name of called) assert.match(lib, new RegExp(`drive::${name},`), name);
});

test("only drive channels open the drive", () => {
  assert.equal(isDriveChannel({ kind: "drive" }), true);
  assert.equal(isDriveChannel({ kind: "text" }), false);
  assert.equal(isDriveChannel(null), false);
});

test("folders come first and names sort without case", () => {
  assert.deepEqual(driveChildren(items, null).map(i => i.id), ["clips", "maps", "c"]);
  assert.deepEqual(driveChildren(items, "maps").map(i => i.id), ["old", "b", "a"]);
  assert.deepEqual(driveChildren(items, "old"), []);
});

test("the open folder falls back to the top when it goes away", () => {
  assert.equal(driveFolderOf(items, "maps"), "maps");
  assert.equal(driveFolderOf(items, "gone"), null);
  assert.equal(driveFolderOf(items, "c"), null, "a file is not a folder");
  assert.equal(driveFolderOf(items, null), null);
});

test("breadcrumbs walk up from the open folder", () => {
  assert.deepEqual(driveCrumbs(items, "old", "files").map(c => c.name), ["files", "Maps", "old"]);
  assert.deepEqual(driveCrumbs(items, null, "files"), [{ id: null, name: "files" }]);
  const loop = [folder("x", "y", "x"), folder("y", "x", "y")];
  assert.equal(driveCrumbs(loop, "x", "files").length, 3, "a loop never hangs");
});

test("a folder can't move inside itself", () => {
  assert.equal(driveInside(items, "old", "maps"), true);
  assert.equal(driveInside(items, "clips", "maps"), false);
  const targets = driveMoveTargets(items, items[0], "files").map(t => t.label);
  assert.deepEqual(targets, ["files", "clips"]);
  const forFile = driveMoveTargets(items, items[4], "files").map(t => t.label);
  assert.deepEqual(forFile, ["files", "clips", "Maps", "Maps / old"]);
});

test("moves stop at the depth limit", () => {
  const deep = [];
  let parent = null;
  for (let i = 0; i < DRIVE_DEPTH_MAX; i++) {
    deep.push(folder(`f${i}`, parent, `f${i}`));
    parent = `f${i}`;
  }
  const loose = folder("loose", null, "loose");
  const labels = driveMoveTargets([...deep, loose], loose, "files").map(t => t.label);
  assert.equal(labels.length, DRIVE_DEPTH_MAX, "every folder but the deepest can take it");
});

test("names are short and can't hold paths", () => {
  assert.equal(driveCleanName("  Maps "), "Maps");
  assert.equal(driveCleanName("a/b"), null);
  assert.equal(driveCleanName("a\\b"), null);
  assert.equal(driveCleanName(".."), null);
  assert.equal(driveCleanName(""), null);
  assert.equal(driveCleanName("x".repeat(DRIVE_NAME_MAX)), "x".repeat(DRIVE_NAME_MAX));
  assert.equal(driveCleanName("x".repeat(DRIVE_NAME_MAX + 1)), null);
});

test("sizes read naturally", () => {
  assert.equal(driveSize(512), "512 B");
  assert.equal(driveSize(2048), "2.0 KB");
  assert.equal(driveSize(5 * 1024 ** 2), "5.0 MB");
  assert.equal(driveSize(3 * 1024 ** 3), "3.00 GB");
  assert.equal(driveSize(null), "");
});

test("each file says who keeps it", () => {
  assert.equal(driveKeptLabel(file("f", null, "f", { kept_here: true, keepers: 1 }), items), "Only on this device");
  assert.equal(driveKeptLabel(file("f", null, "f", { kept_here: true, keepers: 3 }), items), "On this device and 2 others");
  assert.equal(driveKeptLabel(file("f", null, "f", { keepers: 3, keepers_online: 1 }), items), "Kept by 3 people, 1 online");
  const away = file("f", null, "f", { keepers: 2, keepers_online: 0 });
  assert.equal(driveKeptLabel(away, items), "Nobody who has it is online");
  assert.equal(driveUnavailable(away), true);
  assert.equal(driveKeptLabel(items[0], items), "3 items");
  assert.equal(driveKeptLabel(items[1], items), "Empty");
});

test("the drive counts the room left in the server", () => {
  assert.equal(driveRoomLeft({ max_items: 200, used_items: 198 }), 2);
  assert.equal(driveRoomLeft({ max_items: 200, used_items: 201 }), 0);
});
