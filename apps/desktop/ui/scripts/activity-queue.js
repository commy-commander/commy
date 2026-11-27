const QUEUE_MAX = 50;
const QUEUE_TITLE_MAX = 120;
const QUEUE_LINEUP_MAX = QUEUE_MAX;
const DUCK_SHARE = 0.3;
const DUCK_RELEASE_MS = 700;
const DUCK_SETTLE_MS = 1500;
const FULL_VOLUME = 100;

function freshQueue() {
  return { items: [], nextSlot: 1, playingBy: null };
}

function cleanQueueTitle(title) {
  const plain = Array.from(String(title || "").replace(/[\u0000-\u001f\u007f-\u009f]/g, " ").trim());
  return plain.slice(0, QUEUE_TITLE_MAX).join("");
}

function queueSourceFrom(text, allowLinks = true) {
  const value = String(text || "").trim();
  if (!value) return null;
  const video = youtubeVideoId(value);
  if (video) return watchYouTubeSource(video);
  if (allowLinks && isVideoLink(value)) return watchLinkSource(value);
  return null;
}

function sourceIsValid(source) {
  if (!source || typeof source !== "object") return false;
  if (source.youtube) return /^[A-Za-z0-9_-]{11}$/.test(String(source.youtube.video || ""));
  if (source.link) return isVideoLink(source.link.url);
  if (source.attachment) {
    const att = source.attachment.attachment;
    return Boolean(att && att.blob && att.filename && String(att.mime_type || "").startsWith("video/"));
  }
  return false;
}

function sameSource(a, b) {
  return JSON.stringify(a || null) === JSON.stringify(b || null);
}

function queueAddChange(source, title = "") {
  return { queue: { add: { source, title: cleanQueueTitle(title) } } };
}

function queueRemoveChange(slot) {
  return { queue: { remove: { slot } } };
}

function queueSkipChange(runId) {
  return { queue: { skip: { run: runId } } };
}

function queueFinishedChange(runId) {
  return { queue: { finished: { run: runId } } };
}

function queueLineupChange(queue) {
  return { queue: { lineup: { items: queue.items.slice(0, QUEUE_LINEUP_MAX).map(item => ({ ...item })) } } };
}

function queueItemFromWire(item) {
  if (!item || typeof item !== "object" || !Number.isInteger(item.slot) || item.slot < 0) return null;
  if (!sourceIsValid(item.source) || typeof item.by !== "string" || !item.by) return null;
  return { slot: item.slot, source: item.source, title: cleanQueueTitle(item.title), by: item.by };
}

function mayRemoveQueued(run, item, by) {
  return Boolean(run && item) && (by === item.by || by === run.starter);
}

function maySkipQueued(run, queue, by) {
  return Boolean(run) && (by === run.starter || (Boolean(queue.playingBy) && by === queue.playingBy));
}

function queueChangeKind(change) {
  return change && typeof change === "object" ? Object.keys(change)[0] || null : null;
}

function applyQueueChange(run, queue, change, by, keeper) {
  if (!run || run.kind !== "watch") return { ok: false, reason: "no watch" };
  const kind = queueChangeKind(change);
  const body = kind ? change[kind] || {} : {};
  if (kind === "lineup") {
    if (by !== run.starter) return { ok: false, reason: "only the starter keeps the queue" };
    const items = (Array.isArray(body.items) ? body.items : []).slice(0, QUEUE_LINEUP_MAX).map(queueItemFromWire).filter(Boolean);
    queue.items = items;
    queue.nextSlot = Math.max(queue.nextSlot, ...items.map(item => item.slot + 1));
    return { ok: true, reason: null };
  }
  if (!keeper) return { ok: false, reason: null };
  switch (kind) {
    case "add": {
      if (!isParticipant(run, by)) return { ok: false, reason: t("ui.join-to-add-to-the-queue") };
      if (!sourceIsValid(body.source)) return { ok: false, reason: t("ui.paste-a-youtube-link-or-a-direct-https") };
      if (queue.items.length >= QUEUE_MAX) return { ok: false, reason: t("ui.the-queue-is-full") };
      queue.items.push({ slot: queue.nextSlot++, source: body.source, title: cleanQueueTitle(body.title), by });
      return { ok: true, reason: null, broadcast: true };
    }
    case "remove": {
      const at = queue.items.findIndex(item => item.slot === body.slot);
      if (at < 0) return { ok: false, reason: null };
      if (!mayRemoveQueued(run, queue.items[at], by)) return { ok: false, reason: t("ui.only-whoever-added-it-or-the-starter") };
      queue.items.splice(at, 1);
      return { ok: true, reason: null, broadcast: true };
    }
    case "skip": {
      if (body.run !== run.id) return { ok: false, reason: null };
      if (!maySkipQueued(run, queue, by)) return { ok: false, reason: t("ui.only-the-starter-or-whoever-added-this") };
      if (!queue.items.length) return { ok: false, reason: t("ui.nothing-is-queued") };
      return { ok: true, reason: null, advance: true };
    }
    case "finished": {
      if (body.run !== run.id || !isParticipant(run, by) || !queue.items.length) return { ok: false, reason: null };
      return { ok: true, reason: null, advance: true };
    }
    default:
      return { ok: false, reason: null };
  }
}

function nextQueuedActivity(queue, run, now, id) {
  if (!run || !queue.items.length) return null;
  return makeActivity("watch", run.starter, now, id, queue.items[0].source);
}

function continuesQueue(previous, queue, activity) {
  if (!previous || previous.kind !== "watch" || activityKindKey(activity) !== "watch") return false;
  if (activity.starter !== previous.starter || !queue.items.length) return false;
  return sameSource(queue.items[0].source, activity.kind.watch.source);
}

function queueAfterHead(queue) {
  const [head, ...rest] = queue.items;
  return { items: rest, nextSlot: queue.nextSlot, playingBy: head ? head.by : null };
}

function carriedParticipants(previous, activity) {
  return [activity.starter, ...previous.participants.filter(id => id !== activity.starter)];
}

function someoneElseSpeaking(speaking, me) {
  return [...(speaking || [])].some(id => id !== me);
}

function duckedVolume(volume, speaking, me, enabled) {
  const base = Number.isFinite(volume) ? Math.max(0, Math.min(FULL_VOLUME, volume)) : FULL_VOLUME;
  return enabled && someoneElseSpeaking(speaking, me) ? Math.round(base * DUCK_SHARE) : base;
}

function heardVolumeIsTheirs(duck, now) {
  return Boolean(duck) && !duck.ducked && !(now - (duck.setAt ?? -Infinity) < DUCK_SETTLE_MS);
}

function youtubeEndCounts(yt, state) {
  return state === YOUTUBE_STATE.ENDED && Boolean(yt && yt.played);
}

function holdsAtEnd(endedAt, playback) {
  if (endedAt === null || endedAt === undefined) return false;
  return !(playback && Number.isFinite(playback.at) && playback.at > endedAt);
}

function firstEndOf(endedRun, run) {
  return Boolean(run) && endedRun !== run.id;
}

function queueItemLabel(item) {
  if (item.title) return item.title;
  if (item.source && item.source.youtube) return t("ui.youtube-video");
  if (item.source && item.source.link) {
    const file = String(item.source.link.url).split(/[?#]/)[0].split("/").pop();
    try {
      return decodeURIComponent(file) || t("ui.video-2");
    } catch {
      return file || t("ui.video-2");
    }
  }
  if (item.source && item.source.attachment) return item.source.attachment.attachment.filename;
  return t("ui.video-2");
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    QUEUE_MAX, QUEUE_TITLE_MAX, DUCK_SHARE, DUCK_RELEASE_MS, DUCK_SETTLE_MS, FULL_VOLUME, heardVolumeIsTheirs, youtubeEndCounts, firstEndOf, holdsAtEnd,
    freshQueue, cleanQueueTitle, queueSourceFrom, sourceIsValid, sameSource,
    queueAddChange, queueRemoveChange, queueSkipChange, queueFinishedChange, queueLineupChange, queueItemFromWire,
    mayRemoveQueued, maySkipQueued, queueChangeKind, applyQueueChange, nextQueuedActivity, continuesQueue, queueAfterHead,
    carriedParticipants, someoneElseSpeaking, duckedVolume, queueItemLabel,
  };
}
