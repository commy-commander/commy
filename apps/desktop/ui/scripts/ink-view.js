const INK_MAX_POINTS = 64;
const INK_LINE_HOLD_MS = 3000;
const INK_FADE_MS = 800;
const INK_POINT_MS = 1600;
const INK_SEND_EVERY_MS = 40;
const INK_MIN_STEP = 0.002;
const INK_COLORS = ["#ff4d4d", "#ffb547", "#4dd4ff", "#7cff8a", "#c77dff", "#ff7ac6"];
const INK_SETTING_KEY = "commy.streamInk";
const INK_SCREEN_KEY = "commy.streamInkScreen";
const INK_STREAM_KEY = "commy.streamInkAllow";

function inkColor(userId) {
  let hash = 0;
  for (const ch of String(userId || "")) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  return INK_COLORS[hash % INK_COLORS.length];
}

function inkStore() {
  return { lines: new Map(), pings: [] };
}

function inkClamp(v) {
  return Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0;
}

function inkAdd(store, news, now, sharer) {
  const mark = news && news.mark;
  if (!mark) return store;
  const from = news.from;
  if (mark.kind === "clear") {
    if (from === sharer) {
      store.lines.clear();
      store.pings = [];
    } else {
      for (const [key, line] of store.lines) if (line.from === from) store.lines.delete(key);
      store.pings = store.pings.filter(p => p.from !== from);
    }
    return store;
  }
  const points = (mark.points || []).map(([x, y]) => [inkClamp(x), inkClamp(y)]);
  if (!points.length) return store;
  if (mark.kind === "point") {
    store.pings = store.pings.filter(p => p.from !== from);
    store.pings.push({ from, name: news.name || "", x: points[0][0], y: points[0][1], at: now });
    return store;
  }
  if (mark.kind === "line") {
    const key = `${from}:${mark.stroke}`;
    const line = store.lines.get(key) || { from, points: [], at: now };
    line.points.push(...points);
    line.at = now;
    store.lines.delete(key);
    store.lines.set(key, line);
  }
  return store;
}

function inkAlpha(age, hold, fade) {
  if (age <= hold) return 1;
  return Math.max(0, 1 - (age - hold) / fade);
}

function inkFrame(store, now, reducedMotion = false) {
  const fade = reducedMotion ? 1 : INK_FADE_MS;
  const lines = [];
  for (const [key, line] of store.lines) {
    const alpha = inkAlpha(now - line.at, INK_LINE_HOLD_MS, fade);
    if (alpha <= 0) {
      store.lines.delete(key);
      continue;
    }
    lines.push({ color: inkColor(line.from), alpha, points: line.points });
  }
  store.pings = store.pings.filter(p => now - p.at < INK_POINT_MS);
  const pings = store.pings.map(p => ({
    color: inkColor(p.from),
    name: p.name,
    x: p.x,
    y: p.y,
    progress: reducedMotion ? 0 : (now - p.at) / INK_POINT_MS,
    alpha: inkAlpha(now - p.at, INK_POINT_MS - INK_FADE_MS / 2, INK_FADE_MS / 2),
  }));
  return { lines, pings, busy: lines.length > 0 || pings.length > 0 };
}

function inkFit(frameWidth, frameHeight, boxWidth, boxHeight) {
  if (!(frameWidth > 0 && frameHeight > 0 && boxWidth > 0 && boxHeight > 0)) {
    return { x: 0, y: 0, width: Math.max(0, boxWidth || 0), height: Math.max(0, boxHeight || 0) };
  }
  const scale = Math.min(boxWidth / frameWidth, boxHeight / frameHeight);
  const width = frameWidth * scale;
  const height = frameHeight * scale;
  return { x: (boxWidth - width) / 2, y: (boxHeight - height) / 2, width, height };
}

function inkToFrame(px, py, rect) {
  if (!rect || !(rect.width > 0 && rect.height > 0)) return null;
  const x = (px - rect.x) / rect.width;
  const y = (py - rect.y) / rect.height;
  if (x < 0 || y < 0 || x > 1 || y > 1) return null;
  return [x, y];
}

function inkFarEnough(last, next) {
  if (!last) return true;
  return Math.hypot(next[0] - last[0], next[1] - last[1]) >= INK_MIN_STEP;
}

function inkChunks(points, max = INK_MAX_POINTS) {
  const chunks = [];
  for (let i = 0; i < points.length; i += max) chunks.push(points.slice(i, i + max));
  return chunks;
}

function inkPaint(ctx, frame, rect, scale = 1) {
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  for (const line of frame.lines) {
    if (!line.points.length) continue;
    ctx.globalAlpha = line.alpha;
    ctx.strokeStyle = line.color;
    ctx.lineWidth = 4 * scale;
    ctx.beginPath();
    const [x0, y0] = line.points[0];
    ctx.moveTo(rect.x + x0 * rect.width, rect.y + y0 * rect.height);
    if (line.points.length === 1) ctx.lineTo(rect.x + x0 * rect.width + 0.1, rect.y + y0 * rect.height);
    for (const [x, y] of line.points.slice(1)) ctx.lineTo(rect.x + x * rect.width, rect.y + y * rect.height);
    ctx.stroke();
  }
  for (const ping of frame.pings) {
    const cx = rect.x + ping.x * rect.width;
    const cy = rect.y + ping.y * rect.height;
    ctx.globalAlpha = ping.alpha;
    ctx.fillStyle = ping.color;
    ctx.strokeStyle = ping.color;
    ctx.beginPath();
    ctx.arc(cx, cy, 7 * scale, 0, Math.PI * 2);
    ctx.fill();
    ctx.lineWidth = 3 * scale;
    ctx.beginPath();
    ctx.arc(cx, cy, (10 + 26 * ping.progress) * scale, 0, Math.PI * 2);
    ctx.stroke();
    if (ping.name) {
      ctx.font = `600 ${13 * scale}px Figtree, "Segoe UI", sans-serif`;
      const width = ctx.measureText(ping.name).width + 14 * scale;
      ctx.fillStyle = "rgba(11, 13, 23, .82)";
      ctx.fillRect(cx + 12 * scale, cy + 10 * scale, width, 22 * scale);
      ctx.fillStyle = "#f2efe6";
      ctx.fillText(ping.name, cx + 19 * scale, cy + 25 * scale);
    }
  }
  ctx.globalAlpha = 1;
}

function inkSettingOn(saved) {
  return saved === "1";
}

function inkScreenOn(saved) {
  return saved !== "0";
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    INK_MAX_POINTS, INK_LINE_HOLD_MS, INK_FADE_MS, INK_POINT_MS, INK_SEND_EVERY_MS, INK_COLORS,
    INK_SETTING_KEY, INK_SCREEN_KEY, INK_STREAM_KEY,
    inkColor, inkStore, inkAdd, inkFrame, inkFit, inkToFrame, inkFarEnough, inkChunks, inkPaint, inkSettingOn, inkScreenOn,
  };
}
