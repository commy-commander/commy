function queueEntryAsk(queue, ask) {
  return [ask, ...queue.filter(other => other.id !== ask.id)];
}

function entryAsksFromSameDevice(queue, ask) {
  return queue.filter(other => other.id === ask.id || (ask.code && other.code === ask.code));
}

function withoutEntryAsks(queue, gone) {
  const ids = new Set(gone.map(ask => ask.id));
  return queue.filter(ask => !ids.has(ask.id));
}

function withoutEntryAskId(queue, id) {
  return queue.filter(ask => ask.id !== id);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { queueEntryAsk, entryAsksFromSameDevice, withoutEntryAsks, withoutEntryAskId };
}
