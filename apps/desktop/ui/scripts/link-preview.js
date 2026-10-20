const PREVIEW_LINK = /https?:\/\/[^\s<>"']+/gi;
const TRAILING_PUNCTUATION = /[.,!?;:'"\]}]$/;
const WIDE_PREVIEW_MIN_WIDTH = 300;
const WIDE_PREVIEW_MIN_RATIO = 1.2;
const PREVIEW_CARD_WIDTH = 440;
const PREVIEW_IMAGE_MAX_HEIGHT = 240;
const PREVIEW_SIDE_IMAGE = 80;
const LINK_PREVIEW_DEFAULTS = { send: true, show: true };

function trimLinkEnd(url) {
  let end = url;
  for (;;) {
    if (TRAILING_PUNCTUATION.test(end)) end = end.slice(0, -1);
    else if (end.endsWith(")") && end.split("(").length < end.split(")").length) end = end.slice(0, -1);
    else return end;
  }
}

function linksIn(text) {
  return [...String(text || "").matchAll(PREVIEW_LINK)].map(([url]) => trimLinkEnd(url));
}

function previewableLink(text, skip = () => false) {
  for (const url of linksIn(text)) {
    let parsed;
    try {
      parsed = new URL(url);
    } catch {
      continue;
    }
    if (!parsed.hostname || skip(url)) continue;
    return url;
  }
  return null;
}

function previewLayout(preview) {
  const image = preview && preview.image;
  if (!image || !(image.width > 0) || !(image.height > 0)) return "text";
  const wide = image.width >= WIDE_PREVIEW_MIN_WIDTH && image.width / image.height >= WIDE_PREVIEW_MIN_RATIO;
  return wide ? "wide" : "side";
}

function previewImageBox(image, layout) {
  if (layout === "side") return { width: PREVIEW_SIDE_IMAGE, height: PREVIEW_SIDE_IMAGE };
  const inner = PREVIEW_CARD_WIDTH - 30;
  const width = Math.min(inner, image.width);
  return { width, height: Math.min(PREVIEW_IMAGE_MAX_HEIGHT, Math.round((width * image.height) / image.width)) };
}

function previewSite(preview) {
  try {
    return new URL(preview.url).hostname.replace(/^www\./, "");
  } catch {
    return preview.site || "";
  }
}

function imageGridClass(count) {
  return count > 1 ? `image-grid grid-${Math.min(count, 4)}` : "";
}

function readLinkPreviewPrefs(raw) {
  try {
    const saved = JSON.parse(raw || "{}");
    return {
      send: typeof saved.send === "boolean" ? saved.send : LINK_PREVIEW_DEFAULTS.send,
      show: typeof saved.show === "boolean" ? saved.show : LINK_PREVIEW_DEFAULTS.show,
    };
  } catch {
    return { ...LINK_PREVIEW_DEFAULTS };
  }
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { linksIn, previewableLink, previewLayout, previewImageBox, previewSite, imageGridClass, readLinkPreviewPrefs, PREVIEW_CARD_WIDTH, PREVIEW_SIDE_IMAGE, PREVIEW_IMAGE_MAX_HEIGHT };
}
