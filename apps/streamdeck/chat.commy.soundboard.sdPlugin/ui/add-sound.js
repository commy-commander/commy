import { fileStem, preferredUploadServer, soundEmoji, uploadParts, uploadServers } from "./inspector-options.js";

const $ = id => document.getElementById(id);
const WAVE_BARS = 48;
const UPLOAD_WAIT_MS = 45000;

const draft = { bytes: null, buffer: null, sel: null, preview: null, upload: "" };

function option(value, label) {
  const node = document.createElement("option");
  node.value = value;
  node.textContent = label;
  return node;
}

function note(text) {
  $("add-note").textContent = text;
}

function paintSaveButton() {
  const named = $("add-name").value.length >= 2;
  $("add-save").disabled = !draft.buffer || !named || !$("add-server").value || Boolean(draft.upload);
}

function paintSelection() {
  const { buffer, sel } = draft;
  if (!buffer) return;
  for (const [id, value] of [["add-start", sel.start], ["add-end", sel.end]]) {
    $(id).max = String(buffer.duration);
    $(id).value = String(value);
  }
  $("add-times").textContent = `${formatTrimTime(sel.start)} to ${formatTrimTime(sel.end)}, ${(sel.end - sel.start).toFixed(1)} s`;
  const bars = $("add-wave").children;
  for (let i = 0; i < bars.length; i++) {
    const mid = ((i + 0.5) / bars.length) * buffer.duration;
    bars[i].classList.toggle("on", mid >= sel.start && mid <= sel.end);
  }
}

function paintWave(buffer) {
  const channels = Array.from({ length: buffer.numberOfChannels }, (_, i) => buffer.getChannelData(i));
  $("add-wave").replaceChildren(...soundWaveformPeaks(channels, WAVE_BARS).map(peak => {
    const bar = document.createElement("span");
    bar.style.height = `${Math.max(6, Math.round(peak * 100))}%`;
    return bar;
  }));
}

function stopPreview() {
  const preview = draft.preview;
  if (!preview) return;
  draft.preview = null;
  try { preview.source.stop(); } catch {}
  preview.ctx.close().catch(() => {});
  $("add-play").textContent = "Play";
}

function togglePreview() {
  if (draft.preview) return stopPreview();
  const { buffer, sel } = draft;
  if (!buffer) return;
  const ctx = new AudioContext();
  const source = ctx.createBufferSource();
  source.buffer = buffer;
  source.connect(ctx.destination);
  source.start(0, sel.start, sel.end - sel.start);
  const preview = { ctx, source };
  source.onended = () => { if (draft.preview === preview) stopPreview(); };
  draft.preview = preview;
  $("add-play").textContent = "Stop";
}

function moveHandle(which, value) {
  stopPreview();
  draft.sel = moveSoundSelection(draft.sel, which, Number(value), draft.buffer.duration, MAX_SOUND_SECONDS);
  paintSelection();
}

function clearDraft() {
  stopPreview();
  Object.assign(draft, { bytes: null, buffer: null, sel: null });
  $("add-trim").hidden = true;
  paintSaveButton();
}

async function pickFile(file) {
  clearDraft();
  note("");
  if (!file) return;
  if (file.size > SOUND_SOURCE_MAX_BYTES) {
    note(`Pick a file under ${SOUND_SOURCE_MAX_BYTES / 1024 / 1024} MB`);
    return;
  }
  let bytes;
  let buffer;
  try {
    bytes = new Uint8Array(await file.arrayBuffer());
    buffer = await decodeSoundSource(bytes);
  } catch {
    note("Could not read that sound file");
    return;
  }
  if (buffer.duration > SOUND_SOURCE_MAX_SECONDS) {
    note(`Pick a clip under ${SOUND_SOURCE_MAX_SECONDS / 60} minutes`);
    return;
  }
  Object.assign(draft, { bytes, buffer, sel: initialSoundSelection(buffer.duration, MAX_SOUND_SECONDS) });
  if (!$("add-name").value) $("add-name").value = soundNameDraft(fileStem(file.name));
  paintWave(buffer);
  paintSelection();
  $("add-trim").hidden = false;
  note(buffer.duration > MAX_SOUND_SECONDS + 0.2 ? `Sounds can be up to ${MAX_SOUND_SECONDS} seconds. Move Start and End to pick the part you want.` : "");
  paintSaveButton();
}

function toBase64(bytes) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).split(",", 2)[1] ?? "");
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(new Blob([bytes]));
  });
}

export function createAddSound(sendToPlugin) {
  function finished(result) {
    if (!draft.upload || result.upload !== draft.upload) return;
    draft.upload = "";
    if (!result.ok) {
      note(result.message || "Commy could not add that sound");
      paintSaveButton();
      return;
    }
    note(`Added ${result.settings?.sound ?? "the sound"}. This key now plays it.`);
    $("add-file").value = "";
    $("add-name").value = "";
    $("add-emoji").value = "";
    clearDraft();
  }

  async function upload() {
    if (!draft.buffer || draft.upload) return;
    stopPreview();
    let bytes;
    try {
      bytes = await renderTrimmedSound(draft.bytes, draft.buffer, draft.sel);
    } catch {
      note("Could not trim that sound");
      return;
    }
    if (bytes.length > SOUND_STORE_MAX_BYTES) {
      note("That clip is still over 1 MB, pick a shorter part");
      return;
    }
    draft.upload = `${Date.now()}-${Math.random().toString(36).slice(2)}`;
    paintSaveButton();
    note("Uploading...");
    const parts = uploadParts(await toBase64(bytes));
    const { upload: id } = draft;
    sendToPlugin({
      event: "upload-start",
      upload: id,
      server: $("add-server").value,
      name: $("add-name").value,
      emoji: soundEmoji($("add-emoji").value),
      parts: parts.length,
    });
    parts.forEach((data, index) => sendToPlugin({ event: "upload-part", upload: id, index, data }));
    setTimeout(() => finished({ upload: id, ok: false, message: "Commy did not answer, try again" }), UPLOAD_WAIT_MS);
  }

  $("add-file").addEventListener("change", e => void pickFile(e.target.files?.[0]));
  $("add-start").addEventListener("input", e => moveHandle("start", e.target.value));
  $("add-end").addEventListener("input", e => moveHandle("end", e.target.value));
  $("add-play").addEventListener("click", togglePreview);
  $("add-name").addEventListener("input", e => {
    const clean = soundNameDraft(e.target.value);
    if (clean !== e.target.value) e.target.value = clean;
    paintSaveButton();
  });
  $("add-server").addEventListener("change", paintSaveButton);
  $("add-save").addEventListener("click", () => void upload());

  return {
    render(list, settings) {
      const options = uploadServers(list);
      const select = $("add-server");
      const kept = select.value;
      select.replaceChildren(...options.map(o => option(o.value, o.label)));
      select.value = options.some(o => o.value === kept) ? kept : preferredUploadServer(options, settings);
      $("add").hidden = false;
      $("add-locked").hidden = options.length > 0;
      $("add-form").hidden = options.length === 0;
      paintSaveButton();
    },
    finished,
  };
}
