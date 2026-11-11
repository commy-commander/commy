const INK_MAX_POINTS = 64;
const INK_LINE_HOLD_MS = 3000;
const INK_FADE_MS = 800;
const INK_POINT_MS = 1600;
const INK_SEND_EVERY_MS = 40;
const INK_MIN_STEP = 0.002;
const INK_COLORS = ["#f2a468", "#e9c96a", "#69d0c5", "#86d6a0", "#b7a9f7", "#f29bbd"];
const INK_OWN = "#eef0f4";
const INK_TAG_INK = "#0b0d12";
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

function inkFrame(store, now, reducedMotion = false, me = null) {
  const fade = reducedMotion ? 1 : INK_FADE_MS;
  const lines = [];
  for (const [key, line] of store.lines) {
    const alpha = inkAlpha(now - line.at, INK_LINE_HOLD_MS, fade);
    if (alpha <= 0) {
      store.lines.delete(key);
      continue;
    }
    lines.push({ color: inkColor(line.from), own: Boolean(me) && line.from === me, alpha, points: line.points });
  }
  store.pings = store.pings.filter(p => now - p.at < INK_POINT_MS);
  const pings = store.pings.map(p => ({
    color: inkColor(p.from),
    own: Boolean(me) && p.from === me,
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

function inkStudio() {
  if (typeof document === "undefined" || !document.documentElement) return false;
  const data = document.documentElement.dataset || {};
  return data.look === "studio" && data.fit !== "slim";
}

function inkShadow(ctx, y, blur, alpha) {
  ctx.shadowColor = alpha ? `rgba(0, 0, 0, ${alpha})` : "transparent";
  ctx.shadowOffsetX = 0;
  ctx.shadowOffsetY = y;
  ctx.shadowBlur = blur;
}

function inkPen(mark, studio) {
  return studio && mark.own ? INK_OWN : mark.color;
}

function inkTagBox(cx, cy, width, scale = 1) {
  return { x: cx + 12 * scale, y: cy + 12 * scale, width, height: 22 * scale, radii: [6 * scale, 6 * scale, 6 * scale, 2 * scale] };
}

const INK_LAYERS = new WeakMap();

function inkLayer(ctx) {
  if (typeof document === "undefined" || !document.createElement || !ctx || !ctx.canvas || !ctx.canvas.width) return null;
  let layer = INK_LAYERS.get(ctx);
  if (!layer) {
    layer = document.createElement("canvas");
    INK_LAYERS.set(ctx, layer);
  }
  if (layer.width !== ctx.canvas.width || layer.height !== ctx.canvas.height) {
    layer.width = ctx.canvas.width;
    layer.height = ctx.canvas.height;
  }
  const layerCtx = layer.getContext("2d");
  if (!layerCtx) return null;
  layerCtx.setTransform(1, 0, 0, 1, 0, 0);
  layerCtx.clearRect(0, 0, layer.width, layer.height);
  layerCtx.setTransform(ctx.getTransform());
  return layerCtx;
}

function inkStrokeLine(ctx, line, rect, scale, studio) {
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.strokeStyle = inkPen(line, studio);
  ctx.lineWidth = 4 * scale;
  if (studio) inkShadow(ctx, 1 * scale, 3 * scale, 0.65);
  ctx.beginPath();
  const [x0, y0] = line.points[0];
  ctx.moveTo(rect.x + x0 * rect.width, rect.y + y0 * rect.height);
  if (line.points.length === 1) ctx.lineTo(rect.x + x0 * rect.width + 0.1, rect.y + y0 * rect.height);
  for (const [x, y] of line.points.slice(1)) ctx.lineTo(rect.x + x * rect.width, rect.y + y * rect.height);
  ctx.stroke();
}

function inkPaint(ctx, frame, rect, scale = 1, studio = inkStudio()) {
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  for (const line of frame.lines) {
    if (!line.points.length) continue;
    const layer = studio && line.alpha < 1 ? inkLayer(ctx) : null;
    if (layer) {
      layer.globalAlpha = 1;
      inkStrokeLine(layer, line, rect, scale, true);
      ctx.save();
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      inkShadow(ctx, 0, 0, 0);
      ctx.globalAlpha = line.alpha;
      ctx.drawImage(layer.canvas, 0, 0);
      ctx.restore();
      continue;
    }
    ctx.globalAlpha = line.alpha;
    inkStrokeLine(ctx, line, rect, scale, studio);
  }
  for (const ping of frame.pings) {
    const cx = rect.x + ping.x * rect.width;
    const cy = rect.y + ping.y * rect.height;
    const pen = inkPen(ping, studio);
    ctx.globalAlpha = ping.alpha;
    ctx.fillStyle = pen;
    ctx.strokeStyle = pen;
    if (studio) inkShadow(ctx, 1 * scale, 3 * scale, 0.65);
    ctx.beginPath();
    ctx.arc(cx, cy, 7 * scale, 0, Math.PI * 2);
    ctx.fill();
    if (studio) {
      inkShadow(ctx, 0, 0, 0);
      ctx.globalAlpha = ping.alpha * 0.7 * (1 - Math.min(1, Math.max(0, ping.progress)));
    }
    ctx.lineWidth = 3 * scale;
    ctx.beginPath();
    ctx.arc(cx, cy, (10 + 26 * ping.progress) * scale, 0, Math.PI * 2);
    ctx.stroke();
    ctx.globalAlpha = ping.alpha;
    if (ping.name && studio) {
      ctx.font = `700 ${12 * scale}px Onest, system-ui, sans-serif`;
      const box = inkTagBox(cx, cy, ctx.measureText(ping.name).width + 16 * scale, scale);
      inkShadow(ctx, 2 * scale, 6 * scale, 0.45);
      ctx.fillStyle = pen;
      ctx.beginPath();
      if (typeof ctx.roundRect === "function") ctx.roundRect(box.x, box.y, box.width, box.height, box.radii);
      else ctx.rect(box.x, box.y, box.width, box.height);
      ctx.fill();
      inkShadow(ctx, 0, 0, 0);
      ctx.fillStyle = INK_TAG_INK;
      const metrics = ctx.measureText(ping.name);
      const middle = box.y + box.height / 2;
      if (Number.isFinite(metrics.fontBoundingBoxAscent) && Number.isFinite(metrics.fontBoundingBoxDescent)) {
        ctx.fillText(ping.name, box.x + 8 * scale, middle + (metrics.fontBoundingBoxAscent - metrics.fontBoundingBoxDescent) / 2);
      } else {
        ctx.textBaseline = "middle";
        ctx.fillText(ping.name, box.x + 8 * scale, middle);
        ctx.textBaseline = "alphabetic";
      }
    } else if (ping.name) {
      ctx.font = `600 ${13 * scale}px Figtree, "Segoe UI", sans-serif`;
      const width = ctx.measureText(ping.name).width + 14 * scale;
      ctx.fillStyle = "rgba(11, 13, 23, .82)";
      ctx.fillRect(cx + 12 * scale, cy + 10 * scale, width, 22 * scale);
      ctx.fillStyle = "#f2efe6";
      ctx.fillText(ping.name, cx + 19 * scale, cy + 25 * scale);
    }
  }
  if (studio) inkShadow(ctx, 0, 0, 0);
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
    INK_MAX_POINTS, INK_LINE_HOLD_MS, INK_FADE_MS, INK_POINT_MS, INK_SEND_EVERY_MS, INK_COLORS, INK_OWN, INK_TAG_INK,
    INK_SETTING_KEY, INK_SCREEN_KEY, INK_STREAM_KEY,
    inkColor, inkStore, inkAdd, inkFrame, inkFit, inkToFrame, inkFarEnough, inkChunks, inkPaint, inkStudio, inkPen, inkTagBox, inkSettingOn, inkScreenOn,
  };
}
