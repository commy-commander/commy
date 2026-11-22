"use strict";

const WEB_UPDATE_STATUS = {
  current: "web", can_install: false, reason: "web", phase: "idle", version: null, progress: null, checked_at: null, error: null, page: null,
};

const WEB_DEFAULTS = {
  ui_ready: null,
  get_overlay_settings: { enabled: false, corner: "top-left", opacity: 0.9, show_chat: false, show_everyone: false, pictures_only: false, stack_vertical: false, toggle_hotkey: "", chat_hotkey: "" },
  overlay_hotkey_problems: [],
  voice_hotkey_problems: [],
  startup_status: null,
  startup_continue: null,
  get_update_status: WEB_UPDATE_STATUS,
  check_for_updates: WEB_UPDATE_STATUS,
  get_storage_settings: { keep: "90", servers: {} },
  get_autostart: false,
  get_stream_deck: { enabled: false, status: "off", decks: [] },
  speech_voices: { native: false, voices: [] },
  sound_output: false,
  sound_route: null,
  choose_sound: null,
  speech_problem: null,
  get_awaited: 0,
  set_window_title: null,
  set_unread: null,
  get_custom_icon: null,
  take_notification_target: null,
  share_audio_supported: false,
  list_share_sources: [],
  account_cancel_sign_in: null,
};

const WEB_SERVER_WAIT_MS = 20000;
const WEB_SESSION_CHECK_MS = 45_000;
const WEB_SESSION_CHECK_SOONEST_MS = 10_000;

function webSessionCheckDue(lastAt, now) {
  return now - lastAt >= WEB_SESSION_CHECK_SOONEST_MS;
}

const WEB_BYTE_COMMANDS = ["get_attachment_video", "get_voice_message", "download_attachment", "drive_download"];
const WEB_PICKED_COMMANDS = ["attach_dm_file", "attach_file", "drive_upload"];
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

function webSavedImageName(options) {
  const raw = options && options.headers && options.headers["x-commy-name"];
  try {
    return decodeURIComponent(raw || "") || "image";
  } catch {
    return "image";
  }
}

function webNoticeTarget(target) {
  const id = part => /^[0-9a-fA-F-]{1,64}$/.test(part);
  const parts = String(target || "").split(":");
  if (parts[0] === "channel" && parts.length === 3 && id(parts[1]) && id(parts[2])) return parts.join(":");
  if (parts[0] === "dm" && parts.length === 2 && id(parts[1])) return parts.join(":");
  return null;
}

const WEB_JOIN_HASH = "#join=";
const WEB_PENDING_INVITE_KEY = "commy.web.pending-invite";
const WEB_MAX_INVITE_CHARS = 2048;

function webInviteFromHash(hash) {
  if (typeof hash !== "string" || !hash.startsWith(WEB_JOIN_HASH) || hash.length > WEB_JOIN_HASH.length + WEB_MAX_INVITE_CHARS * 3) return null;
  let link;
  try {
    link = new URL(decodeURIComponent(hash.slice(WEB_JOIN_HASH.length)));
  } catch {
    return null;
  }
  const loopback = link.hostname === "localhost" || link.hostname === "127.0.0.1";
  const secure = link.protocol === "https:" || (link.protocol === "http:" && loopback);
  if (!secure || link.username || link.password || !/^\/i\/[A-Za-z0-9-]{3,32}$/.test(link.pathname) || link.search) return null;
  const text = link.href;
  return text.length <= WEB_MAX_INVITE_CHARS ? text : null;
}

function webMovedHome(to) {
  let home;
  try {
    home = new URL(String(to || ""));
  } catch {
    return null;
  }
  if (home.protocol !== "https:" || home.username || home.password || home.port || home.pathname !== "/" || home.search || home.hash) return null;
  return { name: home.hostname, web: `${home.origin}/web/` };
}

function webNotice(args) {
  const clip = (text, chars) => Array.from(String(text || "")).slice(0, chars).join("");
  const target = webNoticeTarget(args && args.target);
  const options = { body: clip(args && args.body, WEB_NOTICE_BODY_CHARS) };
  if (target) options.tag = target;
  return { title: clip(args && args.title, WEB_NOTICE_TITLE_CHARS) || "Commy", options, target };
}

const WEB_SHARE_QUALITY = {
  sharp: { fps: 8, width: 1920, quality: 0.8 },
  balanced: { fps: 12, width: 1600, quality: 0.72 },
  smooth: { fps: 20, width: 1280, quality: 0.62 },
};
const WEB_SHARE_REFUSED = "Screen sharing was not allowed. Press Share Your Screen and pick what to share.";
const WEB_SHARE_UNSUPPORTED = "This browser cannot share its screen. Try Chrome, Edge or Firefox on a computer.";

function webShareQuality(name) {
  return WEB_SHARE_QUALITY[name] || WEB_SHARE_QUALITY.balanced;
}

function webFrameSize(width, height, maxWidth) {
  const w = Math.floor(Number(width) || 0);
  const h = Math.floor(Number(height) || 0);
  if (w <= 0 || h <= 0) return null;
  const scale = Math.min(1, maxWidth / w);
  const even = n => Math.max(2, Math.round(n / 2) * 2);
  return { width: even(w * scale), height: even(h * scale) };
}

function webShareError(e) {
  const name = e && e.name;
  if (name === "NotAllowedError" || name === "AbortError") return WEB_SHARE_REFUSED;
  return `Could not share your screen: ${(e && e.message) || e}`;
}

function webReplyLine(status, text) {
  return `${status}\n${text}`;
}

const WEB_COPY_DB = "commy-web-copy";
const WEB_COPY_STORE = "copies";

function webBrowserCopy(idb) {
  let opening = null;

  function open() {
    if (!idb) return Promise.resolve(null);
    if (!opening) {
      opening = new Promise(resolve => {
        let request;
        try {
          request = idb.open(WEB_COPY_DB, 1);
        } catch {
          resolve(null);
          return;
        }
        request.onupgradeneeded = () => request.result.createObjectStore(WEB_COPY_STORE);
        request.onsuccess = () => {
          const db = request.result;
          db.onversionchange = () => {
            db.close();
            opening = null;
          };
          resolve(db);
        };
        request.onerror = () => resolve(null);
        request.onblocked = () => resolve(null);
      }).then(db => {
        if (!db) opening = null;
        return db;
      });
    }
    return opening;
  }

  function run(mode, work, answer, failed) {
    return open().then(db => {
      if (!db) return failed;
      return new Promise(resolve => {
        try {
          const tx = db.transaction(WEB_COPY_STORE, mode);
          const read = work(tx.objectStore(WEB_COPY_STORE));
          tx.oncomplete = () => resolve(answer(read));
          tx.onerror = () => resolve(failed);
          tx.onabort = () => resolve(failed);
        } catch {
          resolve(failed);
        }
      });
    });
  }

  function load() {
    return run(
      "readonly",
      store => ({ names: store.getAllKeys(), values: store.getAll() }),
      ({ names, values }) => {
        const out = {};
        names.result.forEach((name, i) => {
          if (typeof name === "string" && typeof values.result[i] === "string") out[name] = values.result[i];
        });
        return JSON.stringify(out);
      },
      "{}",
    );
  }

  function write(changesJson) {
    let changes;
    try {
      changes = JSON.parse(changesJson);
    } catch {
      return Promise.resolve(false);
    }
    return run(
      "readwrite",
      store => {
        for (const [name, value] of Object.entries(changes)) {
          if (value === null) store.delete(name);
          else store.put(value, name);
        }
      },
      () => true,
      false,
    );
  }

  function clear() {
    if (!idb) return Promise.resolve(false);
    const held = opening;
    opening = null;
    const closed = held ? held.then(db => db && db.close()) : Promise.resolve();
    return closed.then(
      () =>
        new Promise(resolve => {
          let request;
          try {
            request = idb.deleteDatabase(WEB_COPY_DB);
          } catch {
            resolve(false);
            return;
          }
          request.onsuccess = () => resolve(true);
          request.onerror = () => resolve(false);
        }),
    );
  }

  return { load, write, clear };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { WEB_COPY_DB, WEB_COPY_STORE, webBrowserCopy, WEB_SHARE_QUALITY, WEB_SHARE_REFUSED, WEB_SHARE_UNSUPPORTED, webShareQuality, webFrameSize, webShareError, WEB_SOUND_ACCEPT, WEB_MAX_PICKED_SOUND_BYTES, WEB_SOUND_TOO_LARGE, webSoundName, webCatalogPath, webCatalogRefusal, WEB_IMAGE_ACCEPT, WEB_MAX_PICKED_IMAGE_BYTES, WEB_IMAGE_TOO_LARGE, webReadDataUrl, WEB_BYTE_COMMANDS, WEB_PICKED_COMMANDS, WEB_PICTURE_WAIT_MS, WEB_PICTURE_STORE_MS, WEB_MAX_SEND_BYTES, WEB_SEND_TOO_LARGE, WEB_VIDEO_ACCEPT, webPickFile, webSaveFile, webSavedImageName, WEB_DEFAULTS, WEB_SERVER_WAIT_MS, WEB_SESSION_CHECK_MS, WEB_SESSION_CHECK_SOONEST_MS, webSessionCheckDue, WEB_UNLOCK_TEXT, webDefault, webFileSrc, webSignInUrl, webUnlockText, webReplyLine, webRelays, WEB_NOTICE_ASKED_KEY, webNotice, webNoticeTarget, WEB_PENDING_INVITE_KEY, webInviteFromHash, webMovedHome };
}
