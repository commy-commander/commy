const ACTIVITY_KINDS = [
  { key: "watch", name: "Watch together", blurb: "Play a YouTube link, a video link or an attached video together", icon: "movie", pill: "Watch" },
  { key: "tictactoe", name: "Tic-tac-toe", blurb: "Two play, everyone else in the channel watches", icon: "gamepad", pill: "Game" },
];

const BOARD_CELLS = 9;
const BOARD_LINES = [[0, 1, 2], [3, 4, 5], [6, 7, 8], [0, 3, 6], [1, 4, 7], [2, 5, 8], [0, 4, 8], [2, 4, 6]];
const MARK_NONE = 0;
const MARK_X = 1;
const MARK_O = 2;
const MARK_GLYPHS = { [MARK_X]: "X", [MARK_O]: "O" };
const WATCH_SYNC_TOLERANCE_SECS = 1;
const ACTIVITY_MAX_AVATARS = 8;
const MAX_CHANNEL_VIDEOS = 6;
const VIDEO_LINK_EXTENSIONS = ["mp4", "webm", "ogg"];
const MAX_WATCH_URL_CHARS = 2048;
const YOUTUBE_ORIGIN = "https://www.youtube-nocookie.com";
const YOUTUBE_ID = /^[A-Za-z0-9_-]{11}$/;
const YOUTUBE_HOSTS = ["youtube.com", "www.youtube.com", "m.youtube.com", "music.youtube.com", "youtube-nocookie.com", "www.youtube-nocookie.com"];
const YOUTUBE_PATH_PREFIXES = ["shorts", "live", "embed", "v"];
const YOUTUBE_STATE = { ENDED: 0, PLAYING: 1, PAUSED: 2, BUFFERING: 3, CUED: 5 };
const YOUTUBE_SYNC_TOLERANCE_SECS = 2;
const YOUTUBE_SEEK_JUMP_SECS = 2;

function activityKindKey(activity) {
  const kind = activity && activity.kind;
  if (kind === "tic_tac_toe") return "tictactoe";
  if (kind && typeof kind === "object" && kind.watch) return "watch";
  return null;
}

function activityKindInfo(activity) {
  const key = activityKindKey(activity);
  return ACTIVITY_KINDS.find(k => k.key === key) || null;
}

function activityName(activity) {
  const info = activityKindInfo(activity);
  return info ? info.name : "Activity";
}

function activityPillLabel(activity) {
  const info = activityKindInfo(activity);
  return info ? info.pill : null;
}

function isVideoLink(url) {
  const text = String(url || "");
  if (!text || text.length > MAX_WATCH_URL_CHARS || /\s/.test(text) || !text.startsWith("https://")) return false;
  const path = text.slice("https://".length).split(/[?#]/)[0];
  const slash = path.indexOf("/");
  if (slash <= 0) return false;
  const file = path.slice(path.lastIndexOf("/") + 1);
  const dot = file.lastIndexOf(".");
  if (dot <= 0) return false;
  return VIDEO_LINK_EXTENSIONS.includes(file.slice(dot + 1).toLowerCase());
}

function youtubeVideoId(url) {
  const text = String(url || "").trim();
  if (!text || text.length > MAX_WATCH_URL_CHARS || /\s/.test(text)) return null;
  let parsed;
  try {
    parsed = new URL(/^[a-z]+:\/\//i.test(text) ? text : `https://${text}`);
  } catch {
    return null;
  }
  if (parsed.protocol !== "https:" && parsed.protocol !== "http:") return null;
  const host = parsed.hostname.toLowerCase();
  const parts = parsed.pathname.split("/").filter(Boolean);
  let id = null;
  if (host === "youtu.be" || host === "www.youtu.be") id = parts[0] || null;
  else if (YOUTUBE_HOSTS.includes(host)) {
    if (parts[0] === "watch") id = parsed.searchParams.get("v");
    else if (YOUTUBE_PATH_PREFIXES.includes(parts[0])) id = parts[1] || null;
  }
  return id && YOUTUBE_ID.test(id) ? id : null;
}

function isVideoAttachment(att) {
  return Boolean(att && att.blob && typeof att.mime === "string" && att.mime.startsWith("video/"));
}

function channelVideoAttachments(messages, limit = MAX_CHANNEL_VIDEOS) {
  const seen = new Set();
  const out = [];
  for (const message of [...(messages || [])].reverse()) {
    for (const att of message.attachments || []) {
      if (!isVideoAttachment(att) || seen.has(att.blob)) continue;
      seen.add(att.blob);
      out.push(att);
      if (out.length >= limit) return out;
    }
  }
  return out;
}

function makeActivity(kindKey, starter, now, id, source) {
  const kind = kindKey === "tictactoe" ? "tic_tac_toe" : { watch: { source } };
  return { id, kind, starter, started_at: now };
}

function watchLinkSource(url) {
  return { link: { url: String(url || "").trim() } };
}

function watchAttachmentSource(att) {
  return {
    attachment: {
      attachment: { blob: att.blob, filename: att.filename, size_bytes: att.size_bytes || 0, mime_type: att.mime, provider: att.provider ?? null },
    },
  };
}

function watchYouTubeSource(video) {
  return { youtube: { video } };
}

function watchSource(activity) {
  const kind = activity && activity.kind;
  return kind && typeof kind === "object" && kind.watch ? kind.watch.source : null;
}

function watchLinkUrl(activity) {
  const source = watchSource(activity);
  return source && source.link ? source.link.url : null;
}

function watchAttachment(activity) {
  const source = watchSource(activity);
  if (!source || !source.attachment) return null;
  const att = source.attachment.attachment;
  return { blob: att.blob, filename: att.filename, size_bytes: att.size_bytes, mime: att.mime_type, provider: att.provider ?? null };
}

function watchYouTubeId(activity) {
  const source = watchSource(activity);
  return source && source.youtube && YOUTUBE_ID.test(source.youtube.video) ? source.youtube.video : null;
}

function watchSourceKind(activity) {
  const source = watchSource(activity);
  if (!source) return null;
  if (source.link) return "link";
  if (source.attachment) return "attachment";
  if (source.youtube) return "youtube";
  return null;
}

function watchTitle(activity) {
  if (watchYouTubeId(activity)) return "YouTube video";
  const url = watchLinkUrl(activity);
  if (url) {
    const file = url.split(/[?#]/)[0].split("/").pop();
    return decodeURIComponent(file || url);
  }
  const att = watchAttachment(activity);
  return att ? att.filename : "Video";
}

function youtubeEmbedUrl(video, origin) {
  const params = new URLSearchParams({ enablejsapi: "1", playsinline: "1", rel: "0", modestbranding: "1" });
  if (/^https?:\/\//.test(String(origin || ""))) {
    params.set("origin", origin);
    params.set("widget_referrer", origin);
  }
  return `${YOUTUBE_ORIGIN}/embed/${encodeURIComponent(video)}?${params}`;
}

function youtubeWatchUrl(video) {
  return `https://www.youtube.com/watch?v=${encodeURIComponent(video)}`;
}

function youtubeCommand(func, args = []) {
  return JSON.stringify({ event: "command", func, args, id: 1, channel: "widget" });
}

function youtubeListening() {
  return JSON.stringify({ event: "listening", id: 1, channel: "widget" });
}

function readYouTubeMessage(data) {
  let message = data;
  if (typeof data === "string") {
    try {
      message = JSON.parse(data);
    } catch {
      return null;
    }
  }
  if (!message || typeof message !== "object" || typeof message.event !== "string") return null;
  const info = message.info;
  switch (message.event) {
    case "onReady":
      return { kind: "ready" };
    case "onError":
      return { kind: "error", code: Number(info) };
    case "onStateChange":
      return Number.isInteger(info) ? { kind: "info", state: info } : null;
    case "initialDelivery":
    case "infoDelivery": {
      if (!info || typeof info !== "object") return null;
      const out = { kind: "info" };
      if (Number.isInteger(info.playerState)) out.state = info.playerState;
      if (Number.isFinite(info.currentTime) && info.currentTime >= 0) out.time = info.currentTime;
      if (info.videoData && typeof info.videoData.title === "string" && info.videoData.title.trim()) out.title = info.videoData.title.trim();
      return out.state === undefined && out.time === undefined && out.title === undefined ? null : out;
    }
    default:
      return null;
  }
}

function youtubePlaying(state) {
  return state === YOUTUBE_STATE.PLAYING || state === YOUTUBE_STATE.BUFFERING;
}

function youtubeExpectedTime(seen, now) {
  if (!seen || !Number.isFinite(seen.time)) return null;
  return seen.state === YOUTUBE_STATE.PLAYING ? seen.time + Math.max(0, now - seen.at) / 1000 : seen.time;
}

function youtubeUserSeeked(seen, time, now) {
  const expected = youtubeExpectedTime(seen, now);
  return expected !== null && Number.isFinite(time) && Math.abs(time - expected) > YOUTUBE_SEEK_JUMP_SECS;
}

function youtubeNeedsSeek(current, target) {
  return Math.abs(Number(current) - Number(target)) > YOUTUBE_SYNC_TOLERANCE_SECS;
}

function youtubeErrorText(code) {
  if (code === 100) return "This YouTube video was removed or made private";
  return "YouTube won't play this video here. Open on YouTube still works.";
}

function emptyBoard() {
  return Array(BOARD_CELLS).fill(MARK_NONE);
}

function newActivityRun(activity) {
  const kind = activityKindKey(activity);
  return {
    id: activity.id,
    activity,
    kind,
    starter: activity.starter,
    participants: [activity.starter],
    players: kind === "tictactoe" ? [activity.starter] : [],
    board: emptyBoard(),
    playback: { playing: false, position: 0, at: null },
    seqs: {},
  };
}

function ticTacToeWinner(board) {
  for (const line of BOARD_LINES) {
    const [a, b, c] = line;
    if (board[a] !== MARK_NONE && board[a] === board[b] && board[a] === board[c]) return { mark: board[a], line };
  }
  return null;
}

function boardFull(board) {
  return board.every(cell => cell !== MARK_NONE);
}

function gameOver(board) {
  return Boolean(ticTacToeWinner(board)) || boardFull(board);
}

function turnMark(board) {
  const xs = board.filter(c => c === MARK_X).length;
  const os = board.filter(c => c === MARK_O).length;
  return xs > os ? MARK_O : MARK_X;
}

function markOf(run, userId) {
  const at = run.players.indexOf(userId);
  return at === 0 ? MARK_X : at === 1 ? MARK_O : MARK_NONE;
}

function playerWithMark(run, mark) {
  return run.players[mark - 1] || null;
}

function isParticipant(run, userId) {
  return Boolean(run) && run.participants.includes(userId);
}

function isPlayer(run, userId) {
  return Boolean(run) && run.players.includes(userId);
}

function addParticipant(run, userId) {
  if (run.participants.includes(userId)) return false;
  run.participants.push(userId);
  return true;
}

function seatPlayer(run, userId) {
  if (run.kind !== "tictactoe" || run.players.includes(userId) || run.players.length >= 2) return false;
  run.players.push(userId);
  return true;
}

function removeParticipant(run, userId) {
  const before = run.participants.length + run.players.length;
  run.participants = run.participants.filter(id => id !== userId);
  run.players = run.players.filter(id => id !== userId);
  return run.participants.length + run.players.length !== before;
}

function stale(run, by, seq) {
  if (seq === null || seq === undefined) return false;
  const last = run.seqs[by];
  if (last !== undefined && seq <= last) return true;
  run.seqs[by] = seq;
  return false;
}

function inputKind(input) {
  if (typeof input === "string") return input;
  if (input && typeof input === "object") return Object.keys(input)[0] || null;
  return null;
}

function applyActivityInput(run, input, by, seq) {
  if (!run) return { ok: false, reason: "no activity" };
  if (stale(run, by, seq)) return { ok: false, reason: "stale" };
  const kind = inputKind(input);
  const body = typeof input === "object" && input ? input[kind] : null;
  switch (kind) {
    case "join": {
      const joined = addParticipant(run, by);
      const seated = seatPlayer(run, by);
      return { ok: joined || seated, reason: joined || seated ? null : "already in", seated };
    }
    case "leave":
      if (by === run.starter) return { ok: false, reason: "the starter ends it instead" };
      return { ok: removeParticipant(run, by), reason: null };
    case "playback": {
      if (run.kind !== "watch" || !isParticipant(run, by)) return { ok: false, reason: "not watching" };
      const position = Number(body.position);
      if (!Number.isFinite(position) || position < 0) return { ok: false, reason: "bad position" };
      run.playback = { playing: Boolean(body.playing), position, at: Number(body.at) || null };
      return { ok: true, reason: null };
    }
    case "move": {
      if (run.kind !== "tictactoe") return { ok: false, reason: "not a game" };
      if (run.players.length < 2) return { ok: false, reason: "waiting for a second player" };
      const cell = Number(body.cell);
      if (!Number.isInteger(cell) || cell < 0 || cell >= BOARD_CELLS) return { ok: false, reason: "no such cell" };
      if (gameOver(run.board)) return { ok: false, reason: "the round is over" };
      const mark = markOf(run, by);
      if (mark === MARK_NONE || mark !== turnMark(run.board)) return { ok: false, reason: "not your turn" };
      if (run.board[cell] !== MARK_NONE) return { ok: false, reason: "taken" };
      run.board[cell] = mark;
      return { ok: true, reason: null };
    }
    case "rematch": {
      if (run.kind !== "tictactoe" || !isPlayer(run, by)) return { ok: false, reason: "only a player asks for a rematch" };
      if (!gameOver(run.board)) return { ok: false, reason: "the round is still going" };
      run.board = emptyBoard();
      run.players = [...run.players].reverse();
      return { ok: true, reason: null };
    }
    case "sync": {
      if (by !== run.starter) return { ok: false, reason: "only the starter syncs" };
      const board = Array.isArray(body.board) && body.board.length === BOARD_CELLS ? body.board.map(Number) : emptyBoard();
      run.participants = [run.starter, ...(body.participants || []).filter(id => id !== run.starter)];
      run.players = run.kind === "tictactoe" ? (body.players || []).slice(0, 2) : [];
      run.board = board;
      const position = Number(body.position);
      run.playback = { playing: Boolean(body.playing), position: Number.isFinite(position) && position >= 0 ? position : 0, at: Number(body.at) || null };
      return { ok: true, reason: null };
    }
    default:
      return { ok: false, reason: "unknown input" };
  }
}

function syncInput(run, now) {
  return {
    sync: {
      participants: run.participants.filter(id => id !== run.starter),
      players: run.players,
      board: run.board,
      playing: Boolean(run.playback.playing),
      position: run.playback.position || 0,
      at: run.playback.at || now,
    },
  };
}

function watchTargetPosition(playback, now) {
  if (!playback) return 0;
  if (!playback.playing || !playback.at) return playback.position || 0;
  return (playback.position || 0) + Math.max(0, now - playback.at) / 1000;
}

function watchNeedsSeek(current, target) {
  return Math.abs(Number(current) - Number(target)) > WATCH_SYNC_TOLERANCE_SECS;
}

function activityParticipants(run) {
  if (!run) return [];
  const rest = run.participants.filter(id => !run.players.includes(id));
  return [...run.players, ...rest];
}

function avatarRow(ids, max = ACTIVITY_MAX_AVATARS) {
  const shown = ids.slice(0, max);
  return { shown, more: Math.max(0, ids.length - shown.length) };
}

function gameStatus(run, meId, nameOf) {
  if (!run || run.kind !== "tictactoe") return { text: "", over: false, canRematch: false };
  const who = id => (id === meId ? "You" : nameOf(id));
  if (run.players.length < 2) {
    return { text: isPlayer(run, meId) ? "Waiting for someone to join" : "Press Join to play", over: false, canRematch: false };
  }
  const winner = ticTacToeWinner(run.board);
  if (winner) {
    const user = playerWithMark(run, winner.mark);
    const text = user === meId ? "You win" : `${nameOf(user)} wins`;
    return { text, over: true, canRematch: isPlayer(run, meId), line: winner.line };
  }
  if (boardFull(run.board)) return { text: "Draw", over: true, canRematch: isPlayer(run, meId) };
  const mark = turnMark(run.board);
  const user = playerWithMark(run, mark);
  const text = user === meId ? `Your turn (${MARK_GLYPHS[mark]})` : `${who(user)}'s turn (${MARK_GLYPHS[mark]})`;
  return { text, over: false, canRematch: false, turn: user };
}

function canMoveAt(run, meId, cell) {
  if (!run || run.kind !== "tictactoe" || run.players.length < 2 || gameOver(run.board)) return false;
  return run.board[cell] === MARK_NONE && markOf(run, meId) === turnMark(run.board);
}

function canEndActivity(run, meId, server) {
  if (!run) return false;
  return run.starter === meId || Boolean(server && (server.can_move || server.can_mute));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    ACTIVITY_KINDS, BOARD_CELLS, MARK_NONE, MARK_X, MARK_O, MARK_GLYPHS, WATCH_SYNC_TOLERANCE_SECS, ACTIVITY_MAX_AVATARS,
    activityKindKey, activityKindInfo, activityName, activityPillLabel, isVideoLink, isVideoAttachment, channelVideoAttachments,
    makeActivity, watchLinkSource, watchAttachmentSource, watchLinkUrl, watchAttachment, watchTitle,
    YOUTUBE_ORIGIN, YOUTUBE_STATE, youtubeVideoId, watchYouTubeSource, watchYouTubeId, watchSourceKind, youtubeEmbedUrl, youtubeWatchUrl,
    youtubeCommand, youtubeListening, readYouTubeMessage, youtubePlaying, youtubeExpectedTime, youtubeUserSeeked, youtubeNeedsSeek, youtubeErrorText,
    newActivityRun, ticTacToeWinner, boardFull, gameOver, turnMark, markOf, isParticipant, isPlayer, applyActivityInput, syncInput,
    watchTargetPosition, watchNeedsSeek, activityParticipants, avatarRow, gameStatus, canMoveAt, canEndActivity,
  };
}
