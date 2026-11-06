"use strict";

const WEB_SPEECH_COMMANDS = ["voice_pack_status", "voice_pack_download", "voice_pack_file", "speech_play", "speech_stop"];
const WEB_VOICE_PACK_URL = "scripts/voice-pack.json";
const WEB_VOICE_CACHE = "commy-voices";
const WEB_PROGRESS_MS = 250;

function webPackFile(pack, name) {
  return [...pack.runtime, ...pack.voices.flatMap(v => [v.model, v.config])].find(f => f.name === name) || null;
}

function webPackFilesFor(pack, voice) {
  return [...pack.runtime, voice.model, voice.config];
}

function webPackStatus(pack, saved, available) {
  if (!pack) return { available: false, voices: [] };
  return {
    available,
    voices: pack.voices.map(voice => {
      const needed = webPackFilesFor(pack, voice).filter(f => !(available && saved.get(f.name) === f.size));
      return { id: voice.id, name: voice.name, lang: voice.lang, ready: available && needed.length === 0, download_bytes: needed.reduce((sum, f) => sum + f.size, 0) };
    }),
  };
}

function webHex(buffer) {
  return Array.from(new Uint8Array(buffer), b => b.toString(16).padStart(2, "0")).join("");
}

function webVoicePack(win, emit) {
  let pack = null;
  const busy = new Set();
  const usable = () => Boolean(win.caches && win.crypto && win.crypto.subtle);

  async function manifest() {
    pack ||= win.fetch(WEB_VOICE_PACK_URL, { credentials: "same-origin" }).then(r => (r.ok ? r.json() : null)).catch(() => null);
    const found = await pack;
    if (!found) pack = null;
    return found;
  }

  async function saved() {
    const sizes = new Map();
    if (!usable()) return sizes;
    try {
      const cache = await win.caches.open(WEB_VOICE_CACHE);
      for (const request of await cache.keys()) {
        const name = new URL(request.url).pathname.split("/").pop();
        const response = await cache.match(request);
        sizes.set(name, Number(response && response.headers.get("x-commy-size")));
      }
    } catch {
      return new Map();
    }
    return sizes;
  }

  const key = name => new URL(`/voices/${name}`, win.location.origin).href;

  async function status() {
    return webPackStatus(await manifest(), await saved(), usable());
  }

  async function fetchFile(file, onBytes) {
    let response;
    try {
      response = await win.fetch(file.from, { credentials: "omit" });
    } catch {
      throw "could not download the voice, check your connection";
    }
    if (!response.ok || !response.body) throw "could not download the voice, check your connection";
    const reader = response.body.getReader();
    const bytes = new Uint8Array(file.size);
    let at = 0;
    for (;;) {
      let chunk;
      try {
        chunk = await reader.read();
      } catch {
        throw "the voice download was interrupted";
      }
      if (chunk.done) break;
      if (at + chunk.value.length > file.size) throw "the voice download was not the expected file";
      bytes.set(chunk.value, at);
      at += chunk.value.length;
      onBytes(at);
    }
    if (at !== file.size || webHex(await win.crypto.subtle.digest("SHA-256", bytes)) !== file.sha256) throw "the voice download was not the expected file";
    return bytes;
  }

  async function download(args) {
    const id = args && args.voice;
    const found = await manifest();
    const voice = found && found.voices.find(v => v.id === id);
    if (!voice) throw "unknown voice";
    if (!usable()) throw "voices cannot be saved in this browser";
    const have = await saved();
    const needed = [...new Map(webPackFilesFor(found, voice).map(f => [f.name, f])).values()].filter(f => have.get(f.name) !== f.size);
    if (!needed.length) return null;
    if (busy.has(id)) throw "this voice is already downloading";
    busy.add(id);
    try {
      const cache = await win.caches.open(WEB_VOICE_CACHE);
      const total = needed.reduce((sum, f) => sum + f.size, 0);
      let done = 0;
      let last = 0;
      for (const file of needed) {
        const bytes = await fetchFile(file, loaded => {
          const now = Date.now();
          if (now - last < WEB_PROGRESS_MS) return;
          last = now;
          emit("voice-pack-progress", JSON.stringify({ voice: id, loaded: done + loaded, total }));
        });
        await cache.put(key(file.name), new win.Response(bytes, { headers: { "x-commy-size": String(file.size) } }));
        done += file.size;
      }
      emit("voice-pack-progress", JSON.stringify({ voice: id, loaded: total, total }));
    } catch (e) {
      throw typeof e === "string" ? e : "could not save the voice in this browser";
    } finally {
      busy.delete(id);
    }
    return null;
  }

  async function file(args) {
    const found = await manifest();
    const wanted = found && webPackFile(found, String((args && args.name) || ""));
    if (!wanted) throw "unknown voice file";
    if (!usable()) throw "voices cannot be read in this browser";
    const cache = await win.caches.open(WEB_VOICE_CACHE);
    const response = await cache.match(key(wanted.name));
    if (!response) throw "this voice is not downloaded";
    return response.arrayBuffer();
  }

  return { status, download, file };
}

function webSpeechClip(args, options) {
  const headers = (options && options.headers) || {};
  const rate = Number(headers["x-commy-rate"]);
  if (!(args instanceof Uint8Array) || !Number.isFinite(rate) || rate <= 0) throw "the speech audio has no sample rate";
  const samples = new Float32Array(args.byteLength / 4);
  new Uint8Array(samples.buffer).set(args.subarray(0, samples.length * 4));
  return { samples, rate, append: headers["x-commy-append"] === "1" };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { WEB_SPEECH_COMMANDS, WEB_VOICE_PACK_URL, WEB_VOICE_CACHE, webPackFile, webPackFilesFor, webPackStatus, webHex, webVoicePack, webSpeechClip };
}
