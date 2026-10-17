const MEDIA_HOSTS = [
  "static.klipy.com",
  "images.igdb.com",
  "media.giphy.com",
  /^media[0-9]\.giphy\.com$/,
  "i.giphy.com",
];

const IMAGE_LINK = /https:\/\/[^\s<>"'?]+\.(?:gif|png|jpe?g|webp)(?:\?[^\s<>"']*)?/gi;
const MAX_IMAGE_EMBEDS = 4;

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

if (typeof module !== "undefined" && module.exports) {
  module.exports = { MAX_IMAGE_EMBEDS, isMediaHost, imageHost, imageLinks, embedMode, imageGateText };
}
