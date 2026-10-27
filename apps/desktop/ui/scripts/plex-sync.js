(() => {
  const PLEX_HOST = "app.plex.tv";
  const SETTLE_MS = 800;
  const SCAN_MS = 500;
  const MAX_QUEUED = 20;
  if (location.hostname !== PLEX_HOST || window.__commyWatch) return;
  let video = null;
  let settlingUntil = 0;
  const queued = [];

  function largestVideo() {
    let best = null;
    let bestArea = 0;
    for (const candidate of document.querySelectorAll("video")) {
      const area = candidate.clientWidth * candidate.clientHeight;
      if (area > bestArea) {
        best = candidate;
        bestArea = area;
      }
    }
    return best;
  }

  function report() {
    if (!video || Date.now() < settlingUntil) return;
    queued.push({ playing: !video.paused, position: Number(video.currentTime) || 0 });
    if (queued.length > MAX_QUEUED) queued.shift();
  }

  function follow() {
    const next = largestVideo();
    if (next === video) return;
    if (video) {
      video.removeEventListener("play", report);
      video.removeEventListener("pause", report);
    }
    video = next;
    if (video) {
      video.addEventListener("play", report);
      video.addEventListener("pause", report);
    }
  }

  setInterval(follow, SCAN_MS);
  Object.defineProperty(window, "__commyWatch", {
    value: Object.freeze({
      take() {
        follow();
        return { events: queued.splice(0), video: Boolean(video), playing: Boolean(video && !video.paused) };
      },
      apply(playing) {
        follow();
        if (!video || Boolean(playing) === !video.paused) return;
        settlingUntil = Date.now() + SETTLE_MS;
        if (playing) video.play().catch(() => {});
        else video.pause();
      },
    }),
  });
})();
