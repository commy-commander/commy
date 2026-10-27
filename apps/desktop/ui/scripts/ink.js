"use strict";

const tauri = window.__TAURI__;
const canvas = document.getElementById("ink");
let store = inkStore();
let raf = 0;
let calm = false;

function reducedMotion() {
  let raw = null;
  try { raw = localStorage.getItem(ACCESSIBILITY_KEY); } catch {}
  const os = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  return resolveAccessibility(readAccessibility(raw), { reducedMotion: os }).reducedMotion;
}

function schedule() {
  if (!raf) raf = requestAnimationFrame(paint);
}

function paint() {
  raf = 0;
  const ratio = window.devicePixelRatio || 1;
  const width = Math.round(window.innerWidth * ratio);
  const height = Math.round(window.innerHeight * ratio);
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
  const ctx = canvas.getContext("2d");
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, width, height);
  const frame = inkFrame(store, performance.now(), calm);
  inkPaint(ctx, frame, { x: 0, y: 0, width, height }, ratio * Math.max(1, Math.min(2, window.innerWidth / 1280)));
  if (frame.busy) schedule();
}

function receive(news) {
  if (!news || !news.own) return;
  inkAdd(store, news, performance.now(), news.sharer);
  schedule();
}

async function start() {
  calm = reducedMotion();
  window.addEventListener("storage", e => {
    if (e.key === ACCESSIBILITY_KEY) calm = reducedMotion();
  });
  window.addEventListener("resize", schedule);
  if (!tauri) return;
  await tauri.event.listen("stream-ink", e => receive(e.payload));
  await tauri.event.listen("stream-ink-reset", () => {
    store = inkStore();
    schedule();
  });
}

start();
