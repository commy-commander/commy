const GIPHY_PATH = /^\/(?:media\/(?:[A-Za-z0-9._=\-]{1,500}\/){1,2})?[A-Za-z0-9_\-]{1,80}\.(?:gif|webp)$/;
const TENOR_PATH = /^\/(?:m\/)?[A-Za-z0-9_\-]{1,40}\/[A-Za-z0-9_\-]{1,160}\.(?:gif|webp|png|jpe?g)$/;
const YOUTUBE_THUMBNAIL_PATH = /^\/vi(?:_webp)?\/[A-Za-z0-9_\-]{11}\/[A-Za-z0-9_]{1,40}\.(?:jpg|webp)$/;

const MEDIA_HOSTS = [
  { host: "static.klipy.com", path: /^\/[A-Za-z0-9._~\-\/]{1,400}$/, query: 200 },
  { host: "images.igdb.com", path: /^\/igdb\/image\/upload\/t_[a-z0-9_]{1,40}\/[A-Za-z0-9]{1,32}\.(jpg|png|webp)$/, query: 0 },
  { host: /^(?:media[0-9]?|i)\.giphy\.com$/, path: GIPHY_PATH, query: 300 },
  { host: "cdn3.emoji.gg", path: /^\/emojis\/[A-Za-z0-9_\-]{1,160}\.(?:png|gif)$/, query: 0 },
  { host: /^(?:media[0-9]?|c)\.tenor\.com$/, path: TENOR_PATH, query: 0 },
  { host: /^(?:i[0-9]?\.ytimg|img\.youtube)\.com$/, path: YOUTUBE_THUMBNAIL_PATH, query: 300 },
];

const IMAGE_LINK = /https:\/\/[^\s<>"'?]+\.(?:gif|png|jpe?g|webp)(?:\?[^\s<>"']*)?/gi;
const MAX_IMAGE_EMBEDS = 4;
const MAX_VIDEO_LINK_EMBEDS = 2;
const INLINE_VIDEO_TYPES = new Set(["video/mp4", "video/webm", "video/quicktime", "video/x-m4v", "video/ogg"]);
const MAX_INLINE_VIDEO_BYTES = 100 * 1024 * 1024;
const MAX_AUTO_IMAGE_BYTES = 5 * 1024 * 1024;
const IMAGE_RETRY_AFTER_MS = 30 * 1000;
const VIDEO_RETRIES = 10;

function mediaRule(hostname) {
  const host = String(hostname || "").toLowerCase();
  return MEDIA_HOSTS.find(rule => (typeof rule.host === "string" ? rule.host === host : rule.host.test(host))) || null;
}

function isMediaHost(hostname) {
  return Boolean(mediaRule(hostname));
}

function relaysImage(url) {
  let parsed;
  try {
    parsed = new URL(String(url || ""));
  } catch {
    return false;
  }
  const rule = mediaRule(parsed.hostname);
  if (!rule || parsed.protocol !== "https:" || parsed.username || parsed.password || parsed.port || parsed.hash) return false;
  if (!rule.path.test(parsed.pathname) || parsed.pathname.includes("..")) return false;
  return parsed.search.length <= rule.query;
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

function youtubeThumbnail(id) {
  return `https://i.ytimg.com/vi/${encodeURIComponent(id)}/hqdefault.jpg`;
}

function videoLinkEmbeds(links, videoId, pictured = []) {
  const covered = new Set(pictured.map(videoId).filter(Boolean));
  const embeds = [];
  for (const url of links || []) {
    const id = videoId(url);
    if (!id || covered.has(id)) continue;
    covered.add(id);
    embeds.push({ url, thumbnail: youtubeThumbnail(id) });
    if (embeds.length === MAX_VIDEO_LINK_EMBEDS) break;
  }
  return embeds;
}

function embedMode(url, proxied = true) {
  if (!imageHost(url)) return "none";
  return proxied && relaysImage(url) ? "auto" : "click";
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

const SENT_FILE_GONE = "no longer on this device";

function sentFileGone(error) {
  return String(error || "").toLowerCase().includes(SENT_FILE_GONE);
}

function missingImageText(error) {
  const reason = String(error || "").toLowerCase();
  if (sentFileGone(reason)) return { title: "Image unavailable", hint: "Your copy is no longer on this device.", retry: "Choose File", loading: "Loading...", findAgain: true };
  let hint = "Nobody who has it is online right now.";
  if (reason.includes("not ready")) hint = "Commy is still connecting.";
  else if (reason.includes("too large") || reason.includes("larger than")) hint = "It is too large to show here.";
  else if (reason.includes("not an image")) hint = "This file is not a picture Commy can show.";
  return { title: "Image unavailable", hint, retry: "Try again", loading: "Loading..." };
}

function missingVideoText(error, from) {
  const reason = String(error || "");
  if (sentFileGone(reason)) return { label: "Your copy of this video is no longer on this device", title: "Choose the file again so it plays here and for everyone else.", waiting: false, findAgain: true };
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
  module.exports = { BIG_FILE_BYTES, isBigFile, downloadProgressText, MAX_IMAGE_EMBEDS, MAX_VIDEO_LINK_EMBEDS, youtubeThumbnail, videoLinkEmbeds, MAX_INLINE_VIDEO_BYTES, MAX_AUTO_IMAGE_BYTES, isMediaHost, relaysImage, imageHost, imageLinks, embedMode, imageGateText, loadsImageOnItsOwn, largeImageGateText, retriesImageNow, sentFileGone, missingImageText, missingVideoText, IMAGE_RETRY_AFTER_MS, VIDEO_RETRIES, playsInline };
}
