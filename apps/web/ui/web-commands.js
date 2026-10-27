"use strict";

const WEB_UPDATE_STATUS = {
  current: "web", can_install: false, reason: "web", phase: "idle", version: null, progress: null, checked_at: null, error: null, page: null,
};

const WEB_DEFAULTS = {
  ui_ready: null,
  get_overlay_settings: { enabled: false, corner: "top-left", opacity: 0.9, show_chat: false, show_everyone: false, pictures_only: false, toggle_hotkey: "", chat_hotkey: "" },
  overlay_hotkey_problems: [],
  voice_hotkey_problems: [],
  get_update_status: WEB_UPDATE_STATUS,
  check_for_updates: WEB_UPDATE_STATUS,
  get_integrations: { igdb_client_id: "", igdb_configured: false },
  get_storage_settings: { keep: "90", servers: {} },
  get_autostart: false,
  get_stream_deck: { enabled: false, status: "off", decks: [] },
  speech_voices: { native: false, voices: [] },
  voice_pack_status: { available: false, voices: [] },
  get_awaited: 0,
  set_window_title: null,
  set_unread: null,
  get_custom_icon: null,
  take_pending_invite_link: null,
  list_shareable_emojis: [],
  list_audio_devices: { inputs: [], outputs: [] },
  account_cancel_sign_in: null,
  entry_list_status: { state: "synced", servers: 0, has_key: false, detail: "This browser opens your servers through your other devices." },
};

const WEB_SERVER_WAIT_MS = 20000;

const WEB_BYTE_COMMANDS = ["get_attachment_video", "get_voice_message", "download_attachment"];
const WEB_PICKED_COMMANDS = ["attach_dm_file", "attach_file"];
const WEB_PICTURE_WAIT_MS = 15_000;
const WEB_PICTURE_STORE_MS = 60_000;
const WEB_VIDEO_ACCEPT = "video/mp4,video/quicktime,video/webm,.mp4,.m4v,.mov,.webm";
const WEB_IMAGE_ACCEPT = "image/png,image/jpeg,image/gif,image/webp,.png,.jpg,.jpeg,.gif,.webp";
const WEB_MAX_PICKED_IMAGE_BYTES = 25 * 1024 * 1024;
const WEB_IMAGE_TOO_LARGE = "that image is larger than 25 MB";
const WEB_SOUND_ACCEPT = "audio/*,.mp3,.ogg,.wav,.m4a,.aac,.flac,.opus";
const WEB_MAX_PICKED_SOUND_BYTES = 8 * 1024 * 1024;
const WEB_SOUND_TOO_LARGE = "pick an audio file under 8 MB";
const WEB_MEDIA_PATH = "/api/auth/commy/media";
const WEB_MAX_SEND_BYTES = 100 * 1024 * 1024;
const WEB_SEND_TOO_LARGE = "files larger than 100 MB can be sent from the desktop app";
const WEB_SAVE_LINGER_MS = 60000;
const WEB_NOTICE_TITLE_CHARS = 80;
const WEB_NOTICE_BODY_CHARS = 240;
const WEB_NOTICE_ASKED_KEY = "commy.web.notices-asked";

const WEB_UNLOCK_TEXT = {
  signed_out: "Sign in to open your servers.",
  locked: "This browser could not open your servers yet. Press Try again, or paste your recovery code.",
  asking: "Opening your servers",
  opening: "Opening your servers",
  unlocked: "",
};

function webDefault(cmd) {
  if (!Object.prototype.hasOwnProperty.call(WEB_DEFAULTS, cmd)) return { known: false };
  return { known: true, value: structuredClone(WEB_DEFAULTS[cmd]) };
}

function webFileSrc(path, protocol) {
  return protocol === "commy-media" ? WEB_MEDIA_PATH : path;
}

function webSignInUrl(origin, returnPath) {
  const back = returnPath ? `?next=${encodeURIComponent(returnPath)}` : "";
  return `${origin}/account${back}`;
}

function webUnlockText(view) {
  return (view && view.detail) || WEB_UNLOCK_TEXT[view && view.state] || WEB_UNLOCK_TEXT.locked;
}

function webRelays(doc) {
  const meta = doc.querySelector('meta[name="commy-relays"]');
  const relays = meta ? meta.getAttribute("content").trim() : "";
  return relays || undefined;
}

function webPickFile(doc, kind) {
  return new Promise(resolve => {
    const input = doc.createElement("input");
    input.type = "file";
    if (kind === "video") input.accept = WEB_VIDEO_ACCEPT;
    if (kind === "image") input.accept = WEB_IMAGE_ACCEPT;
    if (kind === "sound") input.accept = WEB_SOUND_ACCEPT;
    input.addEventListener("change", () => resolve((input.files && input.files[0]) || null), { once: true });
    input.addEventListener("cancel", () => resolve(null), { once: true });
    input.click();
  });
}

function webReadDataUrl(file, Reader) {
  return new Promise((resolve, reject) => {
    const reader = new Reader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject("That file could not be read");
    reader.readAsDataURL(file);
  });
}

function webSoundName(filename) {
  const name = String(filename || "");
  const dot = name.lastIndexOf(".");
  return dot < 0 ? "" : name.slice(0, dot);
}

function webCatalogPath(url) {
  return `${WEB_MEDIA_PATH}?u=${encodeURIComponent(url)}`;
}

function webCatalogRefusal(source, res, max) {
  if (res.status === 429) return "Too many images at once, try again in a minute";
  if (!res.ok) return `${source} could not be reached right now (${res.status})`;
  const type = (res.headers.get("content-type") || "").toLowerCase();
  if (!type.startsWith("image/")) return `${source} sent something that is not an image`;
  if (Number(res.headers.get("content-length") || 0) > max) return "that image is too large";
  return null;
}

function webSaveFile(doc, urls, bytes, filename) {
  const url = urls.createObjectURL(new Blob([bytes]));
  const link = doc.createElement("a");
  link.href = url;
  link.download = filename || "file";
  doc.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => urls.revokeObjectURL(url), WEB_SAVE_LINGER_MS);
  return link.download;
}

function webNoticeTarget(target) {
  const id = part => /^[0-9a-fA-F-]{1,64}$/.test(part);
  const parts = String(target || "").split(":");
  if (parts[0] === "channel" && parts.length === 3 && id(parts[1]) && id(parts[2])) return parts.join(":");
  if (parts[0] === "dm" && parts.length === 2 && id(parts[1])) return parts.join(":");
  return null;
}

function webNotice(args) {
  const clip = (text, chars) => Array.from(String(text || "")).slice(0, chars).join("");
  const target = webNoticeTarget(args && args.target);
  const options = { body: clip(args && args.body, WEB_NOTICE_BODY_CHARS) };
  if (target) options.tag = target;
  return { title: clip(args && args.title, WEB_NOTICE_TITLE_CHARS) || "Commy", options, target };
}

function webReplyLine(status, text) {
  return `${status}\n${text}`;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { WEB_SOUND_ACCEPT, WEB_MAX_PICKED_SOUND_BYTES, WEB_SOUND_TOO_LARGE, webSoundName, webCatalogPath, webCatalogRefusal, WEB_IMAGE_ACCEPT, WEB_MAX_PICKED_IMAGE_BYTES, WEB_IMAGE_TOO_LARGE, webReadDataUrl, WEB_BYTE_COMMANDS, WEB_PICKED_COMMANDS, WEB_PICTURE_WAIT_MS, WEB_PICTURE_STORE_MS, WEB_MAX_SEND_BYTES, WEB_SEND_TOO_LARGE, WEB_VIDEO_ACCEPT, webPickFile, webSaveFile, WEB_DEFAULTS, WEB_SERVER_WAIT_MS, WEB_UNLOCK_TEXT, webDefault, webFileSrc, webSignInUrl, webUnlockText, webReplyLine, webRelays, WEB_NOTICE_ASKED_KEY, webNotice, webNoticeTarget };
}
