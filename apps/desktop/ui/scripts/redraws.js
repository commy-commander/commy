function coalescedRuns(work) {
  const running = new Map();
  return key => {
    const run = running.get(key);
    if (run) {
      run.again = true;
      return run.done;
    }
    const fresh = { again: false, done: null };
    running.set(key, fresh);
    fresh.done = (async () => {
      try {
        do {
          fresh.again = false;
          await Promise.resolve(work(key)).catch(e => console.warn(e));
        } while (fresh.again);
      } finally {
        running.delete(key);
      }
    })();
    return fresh.done;
  };
}

const MEMBER_ROWS_STEP = 100;

function memberRowsShown(count, shown) {
  const limit = Math.max(MEMBER_ROWS_STEP, Number(shown) || 0);
  const rows = Math.min(count, limit);
  return { rows, more: count - rows, next: limit + MEMBER_ROWS_STEP };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { coalescedRuns, memberRowsShown, MEMBER_ROWS_STEP };
}
