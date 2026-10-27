const AVATAR_MAX_BYTES = 512 * 1024;

function pictureKind(bytes) {
  const b = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes || []);
  const ascii = (at, text) => [...text].every((ch, i) => b[at + i] === ch.charCodeAt(0));
  if (ascii(0, "GIF87a") || ascii(0, "GIF89a")) return "gif";
  if (b[0] === 0x89 && ascii(1, "PNG")) return "png";
  if (b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff) return "jpeg";
  if (b.length >= 12 && ascii(0, "RIFF") && ascii(8, "WEBP")) return "webp";
  return null;
}

function avatarPicturePlan(bytes, { keepStill = false, max = AVATAR_MAX_BYTES } = {}) {
  const kind = pictureKind(bytes);
  const size = bytes ? bytes.length : 0;
  if (kind === "gif") return size <= max ? "keep" : "too-large";
  if (keepStill && (kind === "png" || kind === "jpeg") && size <= max) return "keep";
  return "redraw";
}

function avatarGifsOffered(settings, serverGifs) {
  return settings && settings.key ? settings.provider === "klipy" : Boolean(serverGifs);
}

if (typeof module !== "undefined") {
  module.exports = { AVATAR_MAX_BYTES, pictureKind, avatarPicturePlan, avatarGifsOffered };
}
