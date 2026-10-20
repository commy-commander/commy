import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  ACTIVITY_KINDS, BOARD_CELLS, MARK_X, MARK_O, MARK_NONE, WATCH_SYNC_TOLERANCE_SECS, ACTIVITY_MAX_AVATARS,
  activityKindKey, activityName, activityPillLabel, isVideoLink, isVideoAttachment, channelVideoAttachments,
  makeActivity, watchLinkSource, watchAttachmentSource, watchLinkUrl, watchAttachment, watchTitle,
  newActivityRun, ticTacToeWinner, boardFull, gameOver, turnMark, markOf, isParticipant, isPlayer, applyActivityInput, syncInput,
  watchTargetPosition, watchNeedsSeek, activityParticipants, avatarRow, gameStatus, canMoveAt, canEndActivity,
} = require("../scripts/activities.js");

const NOW = Date.parse("2026-10-20T12:00:00Z");
const names = { me: "you", m1: "ferris", m2: "nova", m3: "sleepy" };
const nameOf = id => names[id] || "Someone";

const game = (starter = "m1") => makeActivity("tictactoe", starter, NOW, "run-1");
const film = (starter = "m1", url = "https://example.org/clips/final.mp4") => makeActivity("watch", starter, NOW, "run-2", watchLinkSource(url));

function seatedGame(starter = "m1", other = "me") {
  const run = newActivityRun(game(starter));
  assert.equal(applyActivityInput(run, "join", other, 1).ok, true);
  return run;
}

test("the picker offers exactly watch together and tic-tac-toe", () => {
  assert.deepEqual(ACTIVITY_KINDS.map(k => k.key), ["watch", "tictactoe"]);
  assert.deepEqual(ACTIVITY_KINDS.map(k => k.name), ["Watch together", "Tic-tac-toe"]);
  assert.ok(ACTIVITY_KINDS.every(k => k.icon && k.blurb && k.pill));
});

test("the wire shape of an activity names its kind the way the backend spells it", () => {
  assert.deepEqual(game(), { id: "run-1", kind: "tic_tac_toe", starter: "m1", started_at: NOW });
  const watch = film();
  assert.deepEqual(watch.kind, { watch: { source: { link: { url: "https://example.org/clips/final.mp4" } } } });
  assert.equal(activityKindKey(game()), "tictactoe");
  assert.equal(activityKindKey(watch), "watch");
  assert.equal(activityKindKey({ kind: "chess" }), null);
  assert.equal(activityName(game()), "Tic-tac-toe");
  assert.equal(activityName(watch), "Watch together");
  assert.equal(activityName(null), "Activity");
  assert.equal(activityPillLabel(game()), "Game");
  assert.equal(activityPillLabel(watch), "Watch");
  assert.equal(activityPillLabel(null), null);
});

test("only direct https video links are accepted", () => {
  assert.ok(isVideoLink("https://example.org/clips/final.mp4"));
  assert.ok(isVideoLink("https://example.org/a/b.WEBM?token=1#t=10"));
  assert.ok(isVideoLink("https://example.org/talk.ogg"));
  assert.ok(!isVideoLink("http://example.org/final.mp4"));
  assert.ok(!isVideoLink("https://example.org/watch?v=abc"));
  assert.ok(!isVideoLink("https://example.org/final.mkv"));
  assert.ok(!isVideoLink("https://example.org/.mp4"));
  assert.ok(!isVideoLink("https://example.org"));
  assert.ok(!isVideoLink("https:///final.mp4"));
  assert.ok(!isVideoLink("https://example.org/a b.mp4"));
  assert.ok(!isVideoLink(""));
  assert.ok(!isVideoLink(null));
});

test("attached videos in the open channel are offered newest first without repeats", () => {
  const video = (blob, filename = "clip.mp4") => ({ blob, filename, size_bytes: 10, mime: "video/mp4", provider: null });
  const image = { blob: "pic", filename: "pic.png", size_bytes: 1, mime: "image/png" };
  const messages = [
    { attachments: [video("a")] },
    { attachments: [image, video("b", "raid.webm")] },
    { attachments: [] },
    { attachments: [video("a")] },
  ];
  assert.deepEqual(channelVideoAttachments(messages).map(a => a.blob), ["a", "b"]);
  assert.deepEqual(channelVideoAttachments(messages, 1).map(a => a.blob), ["a"]);
  assert.deepEqual(channelVideoAttachments(null), []);
  assert.ok(isVideoAttachment(video("x")) && !isVideoAttachment(image) && !isVideoAttachment(null));
});

test("a watch source carries either the link or the attachment and knows its title", () => {
  const link = film();
  assert.equal(watchLinkUrl(link), "https://example.org/clips/final.mp4");
  assert.equal(watchAttachment(link), null);
  assert.equal(watchTitle(link), "final.mp4");
  const att = { blob: "clip@mock", filename: "clutch-round.mp4", size_bytes: 18400000, mime: "video/mp4", provider: null };
  const attached = makeActivity("watch", "m1", NOW, "run-3", watchAttachmentSource(att));
  assert.deepEqual(attached.kind.watch.source.attachment.attachment, {
    blob: "clip@mock", filename: "clutch-round.mp4", size_bytes: 18400000, mime_type: "video/mp4", provider: null,
  });
  assert.equal(watchLinkUrl(attached), null);
  assert.deepEqual(watchAttachment(attached), att);
  assert.equal(watchTitle(attached), "clutch-round.mp4");
  assert.equal(watchTitle(game()), "Video");
});

test("a new run seats the starter as X and nobody else yet", () => {
  const run = newActivityRun(game("m1"));
  assert.deepEqual(run.participants, ["m1"]);
  assert.deepEqual(run.players, ["m1"]);
  assert.equal(run.board.length, BOARD_CELLS);
  assert.ok(run.board.every(c => c === MARK_NONE));
  assert.equal(markOf(run, "m1"), MARK_X);
  assert.equal(markOf(run, "me"), MARK_NONE);
  const watch = newActivityRun(film("m1"));
  assert.deepEqual(watch.players, []);
  assert.deepEqual(watch.playback, { playing: false, position: 0, at: null });
});

test("the first to join plays O and later joiners spectate", () => {
  const run = newActivityRun(game("m1"));
  assert.deepEqual(applyActivityInput(run, "join", "me", 1), { ok: true, reason: null, seated: true });
  assert.equal(markOf(run, "me"), MARK_O);
  assert.deepEqual(applyActivityInput(run, "join", "m2", 1), { ok: true, reason: null, seated: false });
  assert.deepEqual(run.players, ["m1", "me"]);
  assert.deepEqual(run.participants, ["m1", "me", "m2"]);
  assert.equal(applyActivityInput(run, "join", "m2", 2).ok, false, "joining twice changes nothing");
  assert.ok(isParticipant(run, "m2") && !isPlayer(run, "m2") && isPlayer(run, "me"));
  assert.deepEqual(activityParticipants(run), ["m1", "me", "m2"]);
});

test("a seat frees when its player leaves and the next joiner takes it", () => {
  const run = seatedGame("m1", "me");
  applyActivityInput(run, "join", "m2", 1);
  assert.equal(applyActivityInput(run, "leave", "me", 2).ok, true);
  assert.deepEqual(run.players, ["m1"]);
  assert.deepEqual(run.participants, ["m1", "m2"]);
  assert.equal(applyActivityInput(run, "join", "m3", 1).seated, true);
  assert.deepEqual(run.players, ["m1", "m3"]);
  assert.equal(applyActivityInput(run, "leave", "m1", 1).ok, false, "the starter ends the activity instead of leaving it");
});

test("moves alternate, respect turns, and stop when the round is over", () => {
  const run = seatedGame("m1", "me");
  assert.equal(turnMark(run.board), MARK_X);
  assert.equal(applyActivityInput(run, { move: { cell: 4 } }, "me", 2).reason, "not your turn");
  assert.equal(applyActivityInput(run, { move: { cell: 9 } }, "m1", 1).reason, "no such cell");
  assert.equal(applyActivityInput(run, { move: { cell: 4 } }, "m1", 2).ok, true);
  assert.equal(run.board[4], MARK_X);
  assert.equal(turnMark(run.board), MARK_O);
  assert.equal(applyActivityInput(run, { move: { cell: 4 } }, "me", 3).reason, "taken");
  assert.equal(applyActivityInput(run, { move: { cell: 0 } }, "m2", 1).reason, "not your turn", "spectators never move");
  assert.equal(applyActivityInput(run, { move: { cell: 0 } }, "me", 4).ok, true);
  for (const [cell, by, seq] of [[1, "m1", 3], [8, "me", 5], [7, "m1", 4]]) assert.equal(applyActivityInput(run, { move: { cell } }, by, seq).ok, true);
  assert.deepEqual(ticTacToeWinner(run.board), { mark: MARK_X, line: [1, 4, 7] });
  assert.ok(gameOver(run.board) && !boardFull(run.board));
  assert.equal(applyActivityInput(run, { move: { cell: 2 } }, "me", 6).reason, "the round is over");
  assert.ok(!canMoveAt(run, "me", 2));
});

test("a move needs two players and a game, not a film", () => {
  const solo = newActivityRun(game("m1"));
  assert.equal(applyActivityInput(solo, { move: { cell: 0 } }, "m1", 1).reason, "waiting for a second player");
  const watch = newActivityRun(film("m1"));
  applyActivityInput(watch, "join", "me", 1);
  assert.equal(applyActivityInput(watch, { move: { cell: 0 } }, "m1", 1).reason, "not a game");
  assert.equal(applyActivityInput(watch, "rematch", "m1", 2).ok, false);
});

test("a full board without three in a row is a draw", () => {
  const run = seatedGame("m1", "me");
  const moves = [[0, "m1"], [1, "me"], [2, "m1"], [4, "me"], [3, "m1"], [5, "me"], [7, "m1"], [6, "me"], [8, "m1"]];
  let seqs = { m1: 10, me: 10 };
  for (const [cell, by] of moves) assert.equal(applyActivityInput(run, { move: { cell } }, by, ++seqs[by]).ok, true, `cell ${cell}`);
  assert.equal(ticTacToeWinner(run.board), null);
  assert.ok(boardFull(run.board) && gameOver(run.board));
  assert.deepEqual(gameStatus(run, "me", nameOf), { text: "Draw", over: true, canRematch: true });
  assert.deepEqual(gameStatus(run, "m2", nameOf), { text: "Draw", over: true, canRematch: false });
});

test("a rematch clears the board and hands X to the other player", () => {
  const run = seatedGame("m1", "me");
  assert.equal(applyActivityInput(run, "rematch", "me", 2).reason, "the round is still going");
  for (const [cell, by, seq] of [[0, "m1", 1], [3, "me", 3], [1, "m1", 2], [4, "me", 4], [2, "m1", 3]]) applyActivityInput(run, { move: { cell } }, by, seq);
  assert.equal(ticTacToeWinner(run.board).mark, MARK_X);
  assert.equal(applyActivityInput(run, "rematch", "m2", 1).reason, "only a player asks for a rematch");
  assert.equal(applyActivityInput(run, "rematch", "me", 5).ok, true);
  assert.ok(run.board.every(c => c === MARK_NONE));
  assert.deepEqual(run.players, ["me", "m1"]);
  assert.equal(markOf(run, "me"), MARK_X);
  assert.ok(canMoveAt(run, "me", 4) && !canMoveAt(run, "m1", 4));
});

test("the status line speaks to the viewer", () => {
  const solo = newActivityRun(game("m1"));
  assert.equal(gameStatus(solo, "me", nameOf).text, "Press Join to play");
  assert.equal(gameStatus(solo, "m1", nameOf).text, "Waiting for someone to join");
  const run = seatedGame("m1", "me");
  assert.deepEqual(gameStatus(run, "me", nameOf), { text: "ferris's turn (X)", over: false, canRematch: false, turn: "m1" });
  assert.equal(gameStatus(run, "m1", nameOf).text, "Your turn (X)");
  assert.equal(gameStatus(run, "m2", nameOf).text, "ferris's turn (X)");
  applyActivityInput(run, { move: { cell: 0 } }, "m1", 1);
  assert.equal(gameStatus(run, "me", nameOf).text, "Your turn (O)");
  for (const [cell, by, seq] of [[3, "me", 2], [1, "m1", 2], [4, "me", 3], [2, "m1", 3]]) applyActivityInput(run, { move: { cell } }, by, seq);
  assert.deepEqual(gameStatus(run, "me", nameOf), { text: "ferris wins", over: true, canRematch: true, line: [0, 1, 2] });
  assert.equal(gameStatus(run, "m1", nameOf).text, "You win");
  assert.deepEqual(gameStatus(newActivityRun(film()), "me", nameOf), { text: "", over: false, canRematch: false });
});

test("repeated or reordered inputs from one sender apply once", () => {
  const run = seatedGame("m1", "me");
  assert.equal(applyActivityInput(run, { move: { cell: 0 } }, "m1", 5).ok, true);
  assert.deepEqual(applyActivityInput(run, { move: { cell: 1 } }, "m1", 5), { ok: false, reason: "stale" });
  assert.deepEqual(applyActivityInput(run, { move: { cell: 1 } }, "m1", 4), { ok: false, reason: "stale" });
  assert.equal(run.board.filter(c => c !== MARK_NONE).length, 1);
  assert.equal(applyActivityInput(run, { move: { cell: 1 } }, "me", 5).ok, true, "another sender has its own counter");
  assert.equal(applyActivityInput(null, "join", "me", 1).ok, false);
  assert.equal(applyActivityInput(run, { teleport: {} }, "me", 6).reason, "unknown input");
});

test("playback inputs move every viewer to the same spot", () => {
  const run = newActivityRun(film("m1"));
  assert.equal(applyActivityInput(run, { playback: { playing: true, position: 10, at: NOW } }, "me", 1).reason, "not watching");
  assert.equal(applyActivityInput(run, "join", "me", 2).ok, true);
  assert.equal(applyActivityInput(run, { playback: { playing: true, position: -1, at: NOW } }, "me", 3).reason, "bad position");
  assert.equal(applyActivityInput(run, { playback: { playing: true, position: 10, at: NOW } }, "me", 4).ok, true);
  assert.deepEqual(run.playback, { playing: true, position: 10, at: NOW });
  assert.equal(watchTargetPosition(run.playback, NOW + 2500), 12.5);
  assert.equal(applyActivityInput(run, { playback: { playing: false, position: 30, at: NOW + 3000 } }, "m1", 1).ok, true);
  assert.equal(watchTargetPosition(run.playback, NOW + 60000), 30, "a paused film stays put");
  assert.equal(watchTargetPosition(null, NOW), 0);
  assert.equal(watchTargetPosition({ playing: true, position: 4, at: null }, NOW), 4);
  assert.ok(watchNeedsSeek(10, 11.2) && !watchNeedsSeek(10, 10.9) && !watchNeedsSeek(10, 9.5));
  assert.equal(WATCH_SYNC_TOLERANCE_SECS, 1);
});

test("the starter's sync replaces what a late joiner knows", () => {
  const run = newActivityRun(game("m1"));
  const board = [MARK_X, MARK_O, MARK_NONE, MARK_NONE, MARK_X, MARK_NONE, MARK_NONE, MARK_NONE, MARK_NONE];
  const sync = { sync: { participants: ["m2", "m3"], players: ["m1", "m2"], board, playing: false, position: 0, at: 0 } };
  assert.equal(applyActivityInput(run, sync, "m2", 1).reason, "only the starter syncs");
  assert.equal(applyActivityInput(run, sync, "m1", 1).ok, true);
  assert.deepEqual(run.participants, ["m1", "m2", "m3"]);
  assert.deepEqual(run.players, ["m1", "m2"]);
  assert.deepEqual(run.board, board);
  assert.equal(turnMark(run.board), MARK_O);
  const again = applyActivityInput(run, { sync: { ...sync.sync, participants: ["m1", "m3"], board: [1, 2, 3] } }, "m1", 2);
  assert.equal(again.ok, true);
  assert.deepEqual(run.participants, ["m1", "m3"], "the starter is listed once");
  assert.ok(run.board.every(c => c === MARK_NONE), "a malformed board resets");
});

test("the starter's own sync carries the whole run", () => {
  const run = seatedGame("m1", "me");
  applyActivityInput(run, "join", "m2", 1);
  applyActivityInput(run, { move: { cell: 4 } }, "m1", 1);
  const out = syncInput(run, NOW);
  assert.deepEqual(out, {
    sync: { participants: ["me", "m2"], players: ["m1", "me"], board: run.board, playing: false, position: 0, at: NOW },
  });
  const watch = newActivityRun(film("m1"));
  applyActivityInput(watch, "join", "me", 1);
  applyActivityInput(watch, { playback: { playing: true, position: 42, at: NOW - 1000 } }, "me", 2);
  assert.deepEqual(syncInput(watch, NOW).sync, { participants: ["me"], players: [], board: watch.board, playing: true, position: 42, at: NOW - 1000 });
  const fresh = newActivityRun(game("m1"));
  assert.equal(applyActivityInput(fresh, out, "m1", 1).ok, true);
  assert.deepEqual(fresh.players, ["m1", "me"]);
  assert.deepEqual(fresh.board, run.board);
});

test("avatars show players first and fold the rest into a count", () => {
  const run = newActivityRun(game("m1"));
  for (let i = 0; i < 10; i++) applyActivityInput(run, "join", `u${i}`, 1);
  const order = activityParticipants(run);
  assert.deepEqual(order.slice(0, 2), ["m1", "u0"]);
  assert.equal(order.length, 11);
  const row = avatarRow(order);
  assert.equal(row.shown.length, ACTIVITY_MAX_AVATARS);
  assert.equal(row.more, 3);
  assert.deepEqual(avatarRow(["a", "b"]), { shown: ["a", "b"], more: 0 });
  assert.deepEqual(activityParticipants(null), []);
});

test("the starter or a voice moderator ends an activity", () => {
  const run = newActivityRun(game("m1"));
  assert.ok(canEndActivity(run, "m1", {}));
  assert.ok(!canEndActivity(run, "me", { can_move: false, can_mute: false }));
  assert.ok(canEndActivity(run, "me", { can_move: true }));
  assert.ok(canEndActivity(run, "me", { can_mute: true }));
  assert.ok(!canEndActivity(null, "m1", { can_move: true }));
});

test("the board constants match the backend's cell marks", () => {
  assert.equal(BOARD_CELLS, 9);
  assert.deepEqual([MARK_NONE, MARK_X, MARK_O], [0, 1, 2]);
});
