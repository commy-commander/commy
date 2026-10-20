function channelPinKey(serverId, channelId) {
  return `${serverId}:${channelId}`;
}

function pinnedSet(pinned) {
  if (pinned instanceof Set) return pinned;
  return new Set(Array.isArray(pinned) ? pinned : []);
}

function splitPinned(items, pinned, idOf = item => item.id) {
  const ids = pinnedSet(pinned);
  const result = { pinned: [], rest: [] };
  for (const item of items || []) {
    (ids.has(idOf(item)) ? result.pinned : result.rest).push(item);
  }
  return result;
}

function orderPinned(items, pinned, idOf = item => item.id) {
  const split = splitPinned(items, pinned, idOf);
  return split.pinned.concat(split.rest);
}

function isPinned(pinned, id) {
  return pinnedSet(pinned).has(id);
}

function togglePinned(pinned, id, on) {
  const list = Array.isArray(pinned) ? pinned : [...pinnedSet(pinned)];
  const has = list.includes(id);
  const want = on === undefined ? !has : Boolean(on);
  if (want === has) return list.slice();
  return want ? list.concat(id) : list.filter(x => x !== id);
}

function pinnedChannelIds(pins, serverId) {
  return ((pins && pins.channels) || []).filter(p => p.server === serverId).map(p => p.channel);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { channelPinKey, splitPinned, orderPinned, isPinned, togglePinned, pinnedChannelIds };
}
