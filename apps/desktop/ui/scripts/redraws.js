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

if (typeof module !== "undefined" && module.exports) {
  module.exports = { coalescedRuns };
}
