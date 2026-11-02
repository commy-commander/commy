const MEDIA_HOSTS = [
  "static.klipy.com",
  "images.igdb.com",
  "media.giphy.com",
  /^media[0-9]\.giphy\.com$/,
  "i.giphy.com",
  "cdn3.emoji.gg",
];

const IMAGE_LINK = /https:\/\/[^\s<>"'?]+\.(?:gif|png|jpe?g|webp)(?:\?[^\s<>"']*)?/gi;
const MAX_IMAGE_EMBEDS = 4;
const INLINE_VIDEO_TYPES = new Set(["video/mp4", "video/webm", "video/quicktime", "video/x-m4v", "video/ogg"]);
const MAX_INLINE_VIDEO_BYTES = 100 * 1024 * 1024;
const MAX_AUTO_IMAGE_BYTES = 5 * 1024 * 1024;
const IMAGE_RETRY_AFTER_MS = 30 * 1000;
const VIDEO_RETRIES = 10;

function isMediaHost(hostname) {
  const host = String(hostname || "").toLowerCase();
  return MEDIA_HOSTS.some(rule => (typeof rule === "string" ? rule === host : rule.test(host)));
}

function imageHost(url) {
  try {
    const parsed = new URL(String(url || ""));
    return parsed.protocol === "https:" && parsed.hostname ? parsed.hostname : null;
  } catch {
    return null;
  }
}

function imageLinks(text) {
  const links = [];
  for (const [url] of String(text || "").matchAll(IMAGE_LINK)) {
    if (!imageHost(url)) continue;
    links.push(url);
    if (links.length === MAX_IMAGE_EMBEDS) break;
  }
  return links;
}

function embedMode(url, proxied = true) {
  const host = imageHost(url);
  if (!host) return "none";
  return proxied && isMediaHost(host) ? "auto" : "click";
}

function imageGateText(host) {
  return {
    title: `Image from ${host}`,
    load: "Load",
    loading: "Loading...",
    retry: "Try again",
    hint: `Loading fetches it from ${host}, so that site sees your address.`,
    failed: `The image from ${host} could not be loaded.`,
  };
}

function loadsImageOnItsOwn(att) {
  const size = att ? att.size_bytes : undefined;
  return typeof size === "number" && size >= 0 && size <= MAX_AUTO_IMAGE_BYTES;
}

function largeImageGateText(size) {
  return {
    title: `Image, ${size}`,
    load: "Load",
    loading: "Loading...",
    retry: "Try again",
    hint: "Images over 5 MB load when you click them.",
    failed: "The image could not be loaded.",
  };
}

function retriesImageNow(failure, now) {
  return !failure || now - failure.at >= IMAGE_RETRY_AFTER_MS;
}

function missingImageText(error) {
  const reason = String(error || "").toLowerCase();
  let hint = "Nobody who has it is online right now.";
  if (reason.includes("not ready")) hint = "Commy is still connecting.";
  else if (reason.includes("too large") || reason.includes("larger than")) hint = "It is too large to show here.";
  else if (reason.includes("not an image")) hint = "This file is not a picture Commy can show.";
  return { title: "Image unavailable", hint, retry: "Try again", loading: "Loading..." };
}

function missingVideoText(error, from) {
  const reason = String(error || "");
  if (!reason.toLowerCase().includes("nobody who has")) return { label: `Could not load the video: ${reason}`, title: "", waiting: false };
  const who = from ? `${from} or someone who played it` : "someone who played it";
  return { label: "Nobody who has this video is online", title: `Try again when ${who} is online. Commy also tries again on its own for a few minutes.`, waiting: true };
}

function playsInline(att) {
  if (!att || !INLINE_VIDEO_TYPES.has(String(att.mime || "").toLowerCase())) return false;
  return att.size_bytes > 0 && att.size_bytes <= MAX_INLINE_VIDEO_BYTES;
}

const BIG_FILE_BYTES = 100 * 1024 * 1024;

function isBigFile(size) {
  return Number(size) > BIG_FILE_BYTES;
}

function downloadProgressText(done, total) {
  if (!total) return "Downloading";
  const percent = Math.min(100, Math.floor((done / total) * 100));
  return percent >= 100 ? "Saving" : `Downloading ${percent}%`;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { BIG_FILE_BYTES, isBigFile, downloadProgressText, MAX_IMAGE_EMBEDS, MAX_INLINE_VIDEO_BYTES, MAX_AUTO_IMAGE_BYTES, isMediaHost, imageHost, imageLinks, embedMode, imageGateText, loadsImageOnItsOwn, largeImageGateText, retriesImageNow, missingImageText, missingVideoText, IMAGE_RETRY_AFTER_MS, VIDEO_RETRIES, playsInline };
}
