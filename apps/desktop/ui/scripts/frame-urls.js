const FRAME_URLS_KEPT = 2;

function createFrameUrls({ make = bytes => URL.createObjectURL(new Blob([bytes], { type: "image/jpeg" })), revoke = url => URL.revokeObjectURL(url) } = {}) {
  const slots = new Map();
  return {
    from(slot, base64) {
      const raw = atob(base64);
      const bytes = new Uint8Array(raw.length);
      for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);
      const url = make(bytes);
      const held = slots.get(slot) || [];
      held.push(url);
      while (held.length > FRAME_URLS_KEPT) revoke(held.shift());
      slots.set(slot, held);
      return url;
    },
    drop(slot) {
      for (const url of slots.get(slot) || []) revoke(url);
      slots.delete(slot);
    },
    held(slot) {
      return (slots.get(slot) || []).length;
    },
  };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { createFrameUrls, FRAME_URLS_KEPT };
}
