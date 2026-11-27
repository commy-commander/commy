import "./i18n-global.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const activities = require("../scripts/activities.js");
Object.assign(globalThis, activities);
const {
  QUEUE_MAX, QUEUE_TITLE_MAX, DUCK_SHARE, DUCK_SETTLE_MS, FULL_VOLUME, heardVolumeIsTheirs, youtubeEndCounts, firstEndOf, holdsAtEnd,
  freshQueue, cleanQueueTitle, queueSourceFrom, sourceIsValid, sameSource,
  queueAddChange, queueRemoveChange, queueSkipChange, queueFinishedChange, queueLineupChange, queueItemFromWire,
  mayRemoveQueued, maySkipQueued, applyQueueChange, nextQueuedActivity, continuesQueue, queueAfterHead,
  carriedParticipants, someoneElseSpeaking, duckedVolume, queueItemLabel,
} = require("../scripts/activity-queue.js");
const { makeActivity, newActivityRun, watchYouTubeSource, applyActivityInput } = activities;

const NOW = Date.parse("2026-11-26T12:00:00Z");
const SONG = "dQw4w9WgXcQ";
const OTHER = "a-b_c-d_e-f";

function watching(starter = "host", video = SONG, id = "run-1") {
  const run = newActivityRun(makeActivity("watch", starter, NOW, id, watchYouTubeSource(video)));
  applyActivityInput(run, "join", "guest", 1);
  return run;
}

function added(queue, run, video, by) {
  return applyQueueChange(run, queue, queueAddChange(watchYouTubeSource(video)).queue, by, true);
}

test("queue changes are spelled the way the backend reads them", () => {
  assert.deepEqual(queueAddChange(watchYouTubeSource(SONG), "Song"), { queue: { add: { source: { youtube: { video: SONG } }, title: "Song" } } });
  assert.deepEqual(queueRemoveChange(3), { queue: { remove: { slot: 3 } } });
  assert.deepEqual(queueSkipChange("run-1"), { queue: { skip: { run: "run-1" } } });
  assert.deepEqual(queueFinishedChange("run-1"), { queue: { finished: { run: "run-1" } } });
  const queue = freshQueue();
  queue.items.push({ slot: 1, source: watchYouTubeSource(SONG), title: "", by: "guest" });
  assert.deepEqual(queueLineupChange(queue), { queue: { lineup: { items: [{ slot: 1, source: { youtube: { video: SONG } }, title: "", by: "guest" }] } } });
});

test("titles stay one plain line within the limit", () => {
  assert.equal(cleanQueueTitle("  two\nlines\u0007 "), "two lines");
  assert.equal(Array.from(cleanQueueTitle("é".repeat(QUEUE_TITLE_MAX + 5))).length, QUEUE_TITLE_MAX);
  assert.equal(cleanQueueTitle(null), "");
});

test("only YouTube links and direct video links go in, and the browser takes YouTube only", () => {
  assert.deepEqual(queueSourceFrom(`https://youtu.be/${SONG}`), { youtube: { video: SONG } });
  assert.deepEqual(queueSourceFrom("https://example.org/a.mp4"), { link: { url: "https://example.org/a.mp4" } });
  assert.equal(queueSourceFrom("https://example.org/a.mp4", false), null);
  assert.equal(queueSourceFrom("https://example.org/page"), null);
  assert.equal(queueSourceFrom("  "), null);
  assert.ok(sourceIsValid({ attachment: { attachment: { blob: "b", filename: "clip.mp4", size_bytes: 1, mime_type: "video/mp4" } } }));
  assert.ok(!sourceIsValid({ attachment: { attachment: { blob: "b", filename: "pic.png", size_bytes: 1, mime_type: "image/png" } } }));
  assert.ok(!sourceIsValid({ youtube: { video: "short" } }));
  assert.ok(sameSource(watchYouTubeSource(SONG), { youtube: { video: SONG } }));
  assert.ok(!sameSource(watchYouTubeSource(SONG), watchYouTubeSource(OTHER)));
});

test("the starter's app keeps the queue: joined people add, the full queue refuses", () => {
  const run = watching();
  const queue = freshQueue();
  assert.equal(added(queue, run, SONG, "guest").broadcast, true);
  assert.equal(added(queue, run, SONG, "stranger").ok, false, "only people who joined add");
  assert.deepEqual(queue.items.map(i => [i.slot, i.by]), [[1, "guest"]]);
  while (queue.items.length < QUEUE_MAX) added(queue, run, OTHER, "host");
  const full = added(queue, run, OTHER, "guest");
  assert.equal(full.ok, false);
  assert.ok(full.reason);
  const bad = applyQueueChange(run, freshQueue(), { add: { source: { youtube: { video: "nope" } }, title: "" } }, "guest", true);
  assert.equal(bad.ok, false);
});

test("an app that does not keep the queue only takes the starter's lineup", () => {
  const run = watching();
  const queue = freshQueue();
  assert.equal(added(queue, run, SONG, "guest").ok, true);
  const mine = freshQueue();
  assert.equal(applyQueueChange(run, mine, queueAddChange(watchYouTubeSource(SONG)).queue, "guest", false).ok, false);
  assert.equal(applyQueueChange(run, mine, queueLineupChange(queue).queue, "guest", false).ok, false, "a lineup from someone else is ignored");
  assert.equal(applyQueueChange(run, mine, queueLineupChange(queue).queue, "host", false).ok, true);
  assert.deepEqual(mine.items, queue.items);
  assert.equal(mine.nextSlot, 2);
  const junk = { lineup: { items: [{ slot: 5, source: { youtube: { video: "x" } }, by: "host" }, { slot: "7", source: watchYouTubeSource(SONG), by: "host" }, null] } };
  applyQueueChange(run, mine, junk, "host", false);
  assert.deepEqual(mine.items, []);
  assert.equal(queueItemFromWire({ slot: 1, source: watchYouTubeSource(SONG), title: "a\nb", by: "host" }).title, "a b");
});

test("whoever added an item or the starter removes it", () => {
  const run = watching();
  const queue = freshQueue();
  added(queue, run, SONG, "guest");
  added(queue, run, OTHER, "host");
  const [mine, theirs] = queue.items;
  assert.ok(mayRemoveQueued(run, mine, "guest") && mayRemoveQueued(run, mine, "host"));
  assert.ok(!mayRemoveQueued(run, theirs, "guest"));
  assert.equal(applyQueueChange(run, queue, queueRemoveChange(theirs.slot).queue, "guest", true).ok, false);
  assert.equal(applyQueueChange(run, queue, queueRemoveChange(mine.slot).queue, "guest", true).broadcast, true);
  assert.deepEqual(queue.items.map(i => i.slot), [theirs.slot]);
  assert.equal(applyQueueChange(run, queue, queueRemoveChange(99).queue, "host", true).ok, false);
});

test("skip and finished move on only for the current run, and only with something queued", () => {
  const run = watching();
  const queue = freshQueue();
  assert.equal(applyQueueChange(run, queue, queueSkipChange("run-1").queue, "host", true).ok, false, "nothing queued");
  added(queue, run, OTHER, "guest");
  assert.equal(applyQueueChange(run, queue, queueSkipChange("run-0").queue, "host", true).ok, false, "an older run");
  assert.equal(applyQueueChange(run, queue, queueSkipChange("run-1").queue, "guest", true).ok, false, "guest did not add the playing one");
  assert.equal(applyQueueChange(run, queue, queueSkipChange("run-1").queue, "host", true).advance, true);
  assert.equal(applyQueueChange(run, queue, queueFinishedChange("run-1").queue, "guest", true).advance, true);
  assert.equal(applyQueueChange(run, queue, queueFinishedChange("run-1").queue, "stranger", true).ok, false);
  queue.playingBy = "guest";
  assert.ok(maySkipQueued(run, queue, "guest"), "whoever added the playing one may skip it");
});

test("moving on starts the head of the queue and the next run carries people and the rest", () => {
  const run = watching();
  const queue = freshQueue();
  added(queue, run, OTHER, "guest");
  added(queue, run, SONG, "host");
  const next = nextQueuedActivity(queue, run, NOW + 5, "run-2");
  assert.deepEqual(next, { id: "run-2", kind: { watch: { source: { youtube: { video: OTHER } } } }, starter: "host", started_at: NOW + 5 });
  assert.ok(continuesQueue(run, queue, next));
  assert.ok(!continuesQueue(run, queue, { ...next, starter: "guest" }), "someone else's run is a fresh start");
  assert.ok(!continuesQueue(run, freshQueue(), next));
  assert.ok(!continuesQueue(null, queue, next));
  const after = queueAfterHead(queue);
  assert.deepEqual(after.items.map(i => i.source.youtube.video), [SONG]);
  assert.equal(after.playingBy, "guest");
  assert.equal(after.nextSlot, queue.nextSlot);
  assert.deepEqual(carriedParticipants(run, next), ["host", "guest"]);
  assert.equal(nextQueuedActivity(freshQueue(), run, NOW, "run-3"), null);
});

test("the video turns down while someone else talks and keeps the chosen level otherwise", () => {
  assert.ok(someoneElseSpeaking(new Set(["guest"]), "me"));
  assert.ok(!someoneElseSpeaking(new Set(["me"]), "me"));
  assert.equal(duckedVolume(80, new Set(["guest"]), "me", true), Math.round(80 * DUCK_SHARE));
  assert.equal(duckedVolume(80, new Set(["guest"]), "me", false), 80);
  assert.equal(duckedVolume(80, new Set(), "me", true), 80);
  assert.equal(duckedVolume(undefined, new Set(), "me", true), FULL_VOLUME);
  assert.equal(duckedVolume(250, new Set(), "me", true), FULL_VOLUME);
});

test("queue rows name the video by title, YouTube or file name", () => {
  assert.equal(queueItemLabel({ source: watchYouTubeSource(SONG), title: "Song" }), "Song");
  assert.equal(queueItemLabel({ source: watchYouTubeSource(SONG), title: "" }), "YouTube video");
  assert.equal(queueItemLabel({ source: { link: { url: "https://example.org/clips/my%20clip.mp4?x=1" } }, title: "" }), "my clip.mp4");
});

test("a volume the app just set, or one heard while turned down, never becomes the volume to come back to", () => {
  const now = 10000;
  assert.equal(heardVolumeIsTheirs({ ducked: true, setAt: -Infinity }, now), false);
  assert.equal(heardVolumeIsTheirs({ ducked: false, setAt: now - 200 }, now), false);
  assert.equal(heardVolumeIsTheirs({ ducked: false, setAt: now - DUCK_SETTLE_MS }, now), true);
  assert.equal(heardVolumeIsTheirs({ ducked: false, setAt: -Infinity }, now), true);
  assert.equal(heardVolumeIsTheirs({ ducked: false }, now), true);
  assert.equal(heardVolumeIsTheirs(null, now), false);
});

test("only the video that played here can end the run, and each run ends once", () => {
  const ended = YOUTUBE_STATE.ENDED;
  assert.equal(youtubeEndCounts({ played: false }, ended), false, "a late end from the video before a switch");
  assert.equal(youtubeEndCounts({ played: true }, ended), true);
  assert.equal(youtubeEndCounts({ played: true }, YOUTUBE_STATE.PAUSED), false);
  assert.equal(youtubeEndCounts(null, ended), false);
  assert.equal(firstEndOf(null, { id: "a" }), true);
  assert.equal(firstEndOf("a", { id: "a" }), false);
  assert.equal(firstEndOf("a", { id: "b" }), true);
  assert.equal(firstEndOf(null, null), false);
});

test("a player that reached the end stays there until someone plays or seeks after it ended", () => {
  assert.equal(holdsAtEnd(null, { playing: true, position: 60, at: 1000 }), false);
  assert.equal(holdsAtEnd(5000, { playing: true, position: 0, at: 1000 }), true, "the run still says playing from before the end");
  assert.equal(holdsAtEnd(5000, { playing: false, position: 56, at: 6000 }), false, "the starter's pause at the end comes after it");
  assert.equal(holdsAtEnd(5000, { playing: true, position: 0, at: 9000 }), false, "someone plays it again");
  assert.equal(holdsAtEnd(5000, { playing: true, position: 0, at: null }), true);
});
