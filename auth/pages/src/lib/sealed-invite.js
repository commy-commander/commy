const PREFIX = "sealed1.";
const CONTROL_AND_DIRECTION_OVERRIDES = /[\u0000-\u001f\u007f​-‏‪-‮⁦-⁩]/g;
const NONCE_LEN = 12;
const AAD = new TextEncoder().encode("commy-invite-v1");
const BACKGROUND_AAD = new TextEncoder().encode("commy-invite-bg-v1");
const SHORT_KEY_DOMAIN = new TextEncoder().encode("commy-short-key-v1");
const KEY_RE = /^(?:[A-Za-z0-9_-]{43}|[A-Za-z0-9_-]{22})$/;
const MAX_BACKGROUND_BYTES = 400 * 1024;

function fromBase64Url(text) {
  const b64 = text.replace(/-/g, "+").replace(/_/g, "/") + "=".repeat((4 - (text.length % 4)) % 4);
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

export async function keyBytes(key) {
  const raw = fromBase64Url(key);
  if (key.length === 43) return raw;
  const input = new Uint8Array(SHORT_KEY_DOMAIN.length + raw.length);
  input.set(SHORT_KEY_DOMAIN);
  input.set(raw, SHORT_KEY_DOMAIN.length);
  return new Uint8Array(await crypto.subtle.digest("SHA-256", input));
}

async function open(sealed, key, additionalData) {
  if (!sealed.startsWith(PREFIX) || !KEY_RE.test(key)) return null;
  const bytes = fromBase64Url(sealed.slice(PREFIX.length));
  if (bytes.length < NONCE_LEN + 16) return null;
  const cryptoKey = await crypto.subtle.importKey("raw", await keyBytes(key), "AES-GCM", false, ["decrypt"]);
  return crypto.subtle.decrypt({ name: "AES-GCM", iv: bytes.slice(0, NONCE_LEN), additionalData }, cryptoKey, bytes.slice(NONCE_LEN));
}

export function keyFromHash(hash) {
  const key = hash.replace(/^#/, "");
  return KEY_RE.test(key) ? key : null;
}

export async function openInvite(sealed, key) {
  try {
    const plain = await open(sealed, key, AAD);
    if (!plain) return null;
    const invite = JSON.parse(new TextDecoder().decode(plain));
    if (typeof invite?.n !== "string") return null;
    const name = invite.n.replace(CONTROL_AND_DIRECTION_OVERRIDES, "").trim().slice(0, 100);
    const expires = invite.p === true ? null : Number.isSafeInteger(invite.e) ? invite.e : undefined;
    return { name: name || null, expires };
  } catch {
    return null;
  }
}

function imageType(bytes) {
  if (bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) return "image/jpeg";
  const png = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
  return png.every((b, i) => bytes[i] === b) ? "image/png" : null;
}

export async function openBackground(sealed, key) {
  try {
    const plain = await open(sealed, key, BACKGROUND_AAD);
    if (!plain || plain.byteLength > MAX_BACKGROUND_BYTES) return null;
    const type = imageType(new Uint8Array(plain));
    return type ? new Blob([plain], { type }) : null;
  } catch {
    return null;
  }
}
