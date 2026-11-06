function base64Bytes(base64) {
  const raw = atob(base64);
  const bytes = new Uint8Array(raw.length);
  for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);
  return bytes;
}

function createLatestOnly(run) {
  let busy = false;
  let waiting = null;
  let dropped = 0;
  async function drain(item) {
    busy = true;
    let next = item;
    while (next !== null) {
      waiting = null;
      try {
        await run(next);
      } catch {}
      next = waiting;
    }
    busy = false;
  }
  return {
    push(item) {
      if (busy) {
        if (waiting !== null) dropped++;
        waiting = item;
        return;
      }
      drain(item);
    },
    clear() {
      waiting = null;
    },
    get busy() {
      return busy;
    },
    get dropped() {
      return dropped;
    },
  };
}

function createFramePainter(canvas, { decode = bytes => createImageBitmap(new Blob([bytes], { type: "image/jpeg" })), onPaint = () => {} } = {}) {
  let generation = 0;
  const queue = createLatestOnly(async ({ base64, at }) => {
    const bitmap = await decode(base64Bytes(base64));
    try {
      if (at !== generation) return;
      if (canvas.width !== bitmap.width || canvas.height !== bitmap.height) {
        canvas.width = bitmap.width;
        canvas.height = bitmap.height;
      }
      canvas.getContext("2d").drawImage(bitmap, 0, 0);
      canvas.classList.remove("blank");
      onPaint(bitmap.width, bitmap.height);
    } finally {
      if (bitmap.close) bitmap.close();
    }
  });
  return {
    paint(base64) {
      queue.push({ base64, at: generation });
    },
    clear() {
      generation++;
      queue.clear();
      canvas.width = 0;
      canvas.height = 0;
      canvas.classList.add("blank");
    },
    get dropped() {
      return queue.dropped;
    },
  };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { base64Bytes, createLatestOnly, createFramePainter };
}
