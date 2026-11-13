"use strict";

const IMAGE_COPY_MIME = "image/png";
const IMAGE_NAME_HEADER = "x-commy-name";
const IMAGE_EXTENSIONS = { "image/png": "png", "image/jpeg": "jpg", "image/gif": "gif", "image/webp": "webp", "image/avif": "avif", "image/bmp": "bmp" };
const IMAGE_NAME_UNSAFE = /[\\/:*?"<>|\u0000-\u001f]/g;
const IMAGE_ACTIONS = {
  copy: { menu: "Copy Image", sheet: "Copy Image" },
  save: { menu: "Save Image", sheet: "Save Image" },
  link: { menu: "Copy Image Link", sheet: null },
};
const chatImageSources = new WeakMap();

function markChatImage(img, source) {
  chatImageSources.set(img, source || {});
  return img;
}

function imageLoaded(img) {
  return Boolean(img && img.isConnected && img.complete && img.naturalWidth > 0 && img.getAttribute("src"));
}

function chatImageAt(target) {
  const img = target && target.closest ? target.closest("img") : null;
  return img && chatImageSources.has(img) && imageLoaded(img) ? img : null;
}

function clipboardTakesImages(nav, Item) {
  return Boolean(nav && nav.clipboard && typeof nav.clipboard.write === "function" && typeof Item === "function");
}

function imageActions(source, place, canCopy) {
  const actions = [];
  if (canCopy) actions.push("copy");
  actions.push("save");
  if (source && source.url) actions.push("link");
  return actions.filter(action => IMAGE_ACTIONS[action][place]);
}

function imageNameFromUrl(url) {
  try {
    const parts = new URL(url).pathname.split("/").filter(Boolean);
    return parts.length ? decodeURIComponent(parts[parts.length - 1]) : "";
  } catch {
    return "";
  }
}

function imageSaveName(name, url, mime) {
  const type = String(mime || "").toLowerCase().split(";")[0].trim();
  const ext = IMAGE_EXTENSIONS[type] || "";
  const base = String(name || imageNameFromUrl(url) || "image").replace(IMAGE_NAME_UNSAFE, "_").trim() || "image";
  const dot = base.lastIndexOf(".");
  const hasImageExt = dot > 0 && Object.values(IMAGE_EXTENSIONS).concat("jpeg").includes(base.slice(dot + 1).toLowerCase());
  if (hasImageExt || !ext) return base;
  return `${base}.${ext}`;
}

async function imageBlob(src) {
  const res = await fetch(src);
  if (!res.ok) throw new Error(`the image could not be read (${res.status})`);
  return res.blob();
}

async function imagePng(src) {
  const blob = await imageBlob(src);
  if (blob.type === IMAGE_COPY_MIME) return blob;
  const bitmap = await createImageBitmap(blob);
  const canvas = document.createElement("canvas");
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  canvas.getContext("2d").drawImage(bitmap, 0, 0);
  if (bitmap.close) bitmap.close();
  return new Promise((resolve, reject) => {
    canvas.toBlob(png => (png ? resolve(png) : reject(new Error("the image could not be copied"))), IMAGE_COPY_MIME);
  });
}

function copyImageFailed(source) {
  if (source && source.url) copyText(source.url, "Copied the image link instead");
  else toast("Could not copy the image");
}

function copyChatImage(img) {
  const source = chatImageSources.get(img) || {};
  const png = imagePng(img.src);
  png.catch(() => {});
  let item;
  try {
    item = new ClipboardItem({ [IMAGE_COPY_MIME]: png });
  } catch {
    copyImageFailed(source);
    return Promise.resolve(false);
  }
  return navigator.clipboard.write([item]).then(
    () => {
      toast("Image copied");
      return true;
    },
    () => {
      copyImageFailed(source);
      return false;
    },
  );
}

async function saveImageBytes(img, source) {
  const blob = await imageBlob(img.src);
  const name = imageSaveName(source.name, source.url, blob.type);
  const bytes = new Uint8Array(await blob.arrayBuffer());
  return invoke("save_image", bytes, { headers: { [IMAGE_NAME_HEADER]: encodeURIComponent(name) } });
}

async function saveChatImage(img) {
  const source = chatImageSources.get(img) || {};
  if (source.att && source.att.filename) return saveAttachment(source.att);
  try {
    const saved = await saveImageBytes(img, source);
    toast(saved ? `Saved to ${saved}` : "Download cancelled");
    return Boolean(saved);
  } catch (e) {
    toast(source.url ? "Could not save this image. Open its link to save it from there." : String(e));
    return false;
  }
}

function runImageAction(action, img) {
  if (action === "copy") return copyChatImage(img);
  if (action === "save") return saveChatImage(img);
  const source = chatImageSources.get(img) || {};
  return copyText(source.url, "Image link copied");
}

function chatImageMenuItems(item, img) {
  const source = chatImageSources.get(img) || {};
  const actions = imageActions(source, "menu", clipboardTakesImages(navigator, typeof ClipboardItem === "undefined" ? null : ClipboardItem));
  for (const action of actions) item(IMAGE_ACTIONS[action].menu, () => runImageAction(action, img));
  return actions.length;
}

function chatImageSheetActions(img) {
  const source = chatImageSources.get(img) || {};
  const canCopy = clipboardTakesImages(navigator, typeof ClipboardItem === "undefined" ? null : ClipboardItem);
  return imageActions(source, "sheet", canCopy).map(action => ({ action, title: IMAGE_ACTIONS[action].sheet, run: () => runImageAction(action, img) }));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { IMAGE_COPY_MIME, IMAGE_NAME_HEADER, IMAGE_ACTIONS, clipboardTakesImages, imageActions, imageNameFromUrl, imageSaveName };
}
